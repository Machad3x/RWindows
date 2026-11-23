//! Virtualized uniform grid for the Start menu's pinned apps. Same job as
//! [`crate::list_view`] but with a fixed number of columns.

use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;

use crate::gfx::rect;

/// Win11 Start pinned tile: 72×72 DIPs with 8 DIPs between tiles.
pub const CELL: f32 = 72.0;
/// Space between tiles.
pub const GAP: f32 = 8.0;
/// Number of pinned columns in Windows 11 Start.
pub const COLUMNS: usize = 6;

/// Geometry of a uniform, scrollable grid. Items are indices `0..count`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    count: usize,
    columns: usize,
    cell: f32,
    gap: f32,
    viewport: f32,
    offset: f32,
}

impl Grid {
    /// A grid with `count` items and no viewport set.
    pub fn new(count: usize) -> Self {
        Self { count, columns: COLUMNS, cell: CELL, gap: GAP, viewport: 0.0, offset: 0.0 }
    }

    /// Sets the visible height and clamps the scroll offset.
    pub fn set_viewport(&mut self, height: f32) {
        self.viewport = height.max(0.0);
        self.scroll_to(self.offset);
    }

    /// Scroll offset in DIPs.
    pub fn offset(&self) -> f32 {
        self.offset
    }

    /// Total content height.
    pub fn total_height(&self) -> f32 {
        let rows = self.count.div_ceil(self.columns);
        rows as f32 * (self.cell + self.gap)
    }

    /// Clamps `offset` to the content.
    pub fn scroll_to(&mut self, offset: f32) {
        let max = (self.total_height() - self.viewport).max(0.0);
        self.offset = offset.clamp(0.0, max);
    }

    /// Rectangle of item `index`, relative to the viewport origin.
    pub fn cell_rect(&self, index: usize, left: f32, top: f32) -> D2D_RECT_F {
        let row = index / self.columns;
        let col = index % self.columns;
        let x = left + col as f32 * (self.cell + self.gap);
        let y = top + row as f32 * (self.cell + self.gap) - self.offset;
        rect(x, y, x + self.cell, y + self.cell)
    }

    /// Indices that can touch the viewport, with one extra row of overscan.
    pub fn visible(&self) -> std::ops::Range<usize> {
        if self.count == 0 || self.viewport <= 0.0 {
            return 0..0;
        }
        let row_h = self.cell + self.gap;
        let first_row = (self.offset / row_h) as usize;
        let last_row = ((self.offset + self.viewport) / row_h).ceil() as usize + 1;
        (first_row * self.columns).min(self.count)..(last_row * self.columns).min(self.count)
    }

    /// Rects of the virtualized cells.
    pub fn arrange(&self, left: f32, top: f32) -> Vec<(usize, D2D_RECT_F)> {
        self.visible().map(|i| (i, self.cell_rect(i, left, top))).collect()
    }

    /// Index under a point in viewport coordinates.
    pub fn hit(&self, x: f32, y: f32, left: f32, top: f32) -> Option<usize> {
        if x < left || y < top {
            return None;
        }
        let pitch = self.cell + self.gap;
        let dx = x - left;
        let dy = y - top + self.offset;
        // The gap between tiles is not part of either tile.
        if dx % pitch > self.cell || dy % pitch > self.cell {
            return None;
        }
        let col = (dx / pitch) as usize;
        let row = (dy / pitch) as usize;
        if col >= self.columns {
            return None;
        }
        let index = row * self.columns + col;
        (index < self.count).then_some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Grid {
        let mut g = Grid::new(18);
        g.set_viewport(160.0);
        g
    }

    #[test]
    fn cell_positions_should_follow_columns_and_gap() {
        let g = grid();
        let a = g.cell_rect(0, 10.0, 20.0);
        let b = g.cell_rect(1, 10.0, 20.0);
        let c = g.cell_rect(6, 10.0, 20.0);
        assert_eq!((a.left, a.top, a.right, a.bottom), (10.0, 20.0, 82.0, 92.0));
        assert_eq!(b.left, 90.0);
        assert_eq!(c.top, 100.0);
    }

    #[test]
    fn visible_rows_should_include_one_row_of_overscan() {
        let g = grid();
        assert_eq!(g.visible(), 0..18);
    }

    #[test]
    fn hit_should_find_the_tile_under_the_point() {
        let g = grid();
        assert_eq!(g.hit(20.0, 30.0, 10.0, 20.0), Some(0));
        assert_eq!(g.hit(95.0, 30.0, 10.0, 20.0), Some(1));
        assert_eq!(g.hit(20.0, 110.0, 10.0, 20.0), Some(6));
        assert_eq!(g.hit(700.0, 30.0, 10.0, 20.0), None);
        // 8 DIP gap after the first tile (tile ends at x=82 from origin 10).
        assert_eq!(g.hit(86.0, 30.0, 10.0, 20.0), None);
    }

    #[test]
    fn scroll_should_be_clamped_to_the_last_row() {
        let mut g = grid();
        g.scroll_to(10_000.0);
        assert_eq!(g.offset(), 80.0);
    }
}
