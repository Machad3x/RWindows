//! Single-axis layout of measured children, like WinUI's `StackPanel`.
//!
//! [`Stack`] takes the desired `(width, height)` of each child and returns its
//! `D2D_RECT_F`. It does no text measurement: controls measure themselves
//! (`Button::desired_size` and friends); layout only positions them. Sizes are
//! finite DIPs from text measurement; NaN/infinity is not validated.
//!
//! Sources: [`StackPanel`](https://learn.microsoft.com/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.stackpanel)
//! (orientation, `Spacing` between stacked items) and
//! [`FrameworkElement.HorizontalAlignment`](https://learn.microsoft.com/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.frameworkelement.horizontalalignment)
//! (an explicitly measured child keeps its own cross-axis size, like Left/Top).

use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;

use crate::gfx::rect;

/// The axis a [`Stack`] arranges its children along, like `StackPanel.Orientation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    /// Left to right (`Orientation=Horizontal`).
    Horizontal,
    /// Top to bottom, the `StackPanel` default (`Orientation=Vertical`).
    Vertical,
}

/// A single-axis stack with `StackPanel.Spacing` between adjacent children.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stack {
    orientation: Orientation,
    spacing: f32,
}

impl Stack {
    /// A stack along `orientation` with no spacing.
    pub const fn new(orientation: Orientation) -> Self {
        Self { orientation, spacing: 0.0 }
    }

    /// Sets the uniform gap between adjacent children (`StackPanel.Spacing`).
    pub const fn with_spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    /// Sums the stack axis and takes the largest cross-axis size.
    pub fn desired_size(&self, sizes: &[(f32, f32)]) -> (f32, f32) {
        let mut main: f32 = 0.0;
        let mut cross: f32 = 0.0;
        for &(w, h) in sizes {
            let (a, b) = match self.orientation {
                Orientation::Horizontal => (w, h),
                Orientation::Vertical => (h, w),
            };
            main += a;
            cross = cross.max(b);
        }
        if sizes.len() > 1 {
            main += self.spacing * (sizes.len() - 1) as f32;
        }
        match self.orientation {
            Orientation::Horizontal => (main, cross),
            Orientation::Vertical => (cross, main),
        }
    }

    /// Returns one rect per child at (`left`, `top`), in the same order as `sizes`.
    pub fn arrange(&self, sizes: &[(f32, f32)], left: f32, top: f32) -> Vec<D2D_RECT_F> {
        let mut rects = Vec::with_capacity(sizes.len());
        let mut start = match self.orientation {
            Orientation::Horizontal => left,
            Orientation::Vertical => top,
        };
        for &(w, h) in sizes {
            rects.push(match self.orientation {
                Orientation::Horizontal => rect(start, top, start + w, top + h),
                Orientation::Vertical => rect(left, start, left + w, start + h),
            });
            let size = match self.orientation {
                Orientation::Horizontal => w,
                Orientation::Vertical => h,
            };
            start += size + self.spacing;
        }
        rects
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(r: D2D_RECT_F) -> (f32, f32, f32, f32) {
        (r.left, r.top, r.right, r.bottom)
    }

    #[test]
    fn horizontal_stack_should_place_children_left_to_right_with_spacing() {
        let stack = Stack::new(Orientation::Horizontal).with_spacing(8.0);
        let rects = stack.arrange(&[(40.0, 32.0), (60.0, 32.0), (20.0, 32.0)], 10.0, 20.0);
        let got: Vec<_> = rects.into_iter().map(bounds).collect();
        assert_eq!(got, vec![
            (10.0, 20.0, 50.0, 52.0),
            (58.0, 20.0, 118.0, 52.0),
            (126.0, 20.0, 146.0, 52.0),
        ]);
    }

    #[test]
    fn vertical_stack_should_place_children_top_to_bottom_with_spacing() {
        let stack = Stack::new(Orientation::Vertical).with_spacing(4.0);
        let rects = stack.arrange(&[(100.0, 20.0), (100.0, 30.0)], 10.0, 20.0);
        let got: Vec<_> = rects.into_iter().map(bounds).collect();
        assert_eq!(got, vec![(10.0, 20.0, 110.0, 40.0), (10.0, 44.0, 110.0, 74.0)]);
    }

    #[test]
    fn desired_size_should_sum_the_stack_axis_and_take_the_max_on_the_other() {
        let sizes = [(40.0, 32.0), (60.0, 48.0), (20.0, 24.0)];
        assert_eq!(Stack::new(Orientation::Horizontal).with_spacing(8.0).desired_size(&sizes), (136.0, 48.0));
        assert_eq!(Stack::new(Orientation::Vertical).with_spacing(4.0).desired_size(&sizes), (60.0, 112.0));
    }

    #[test]
    fn a_single_child_should_ignore_spacing() {
        let stack = Stack::new(Orientation::Horizontal).with_spacing(8.0);
        assert_eq!(stack.desired_size(&[(40.0, 32.0)]), (40.0, 32.0));
        let rects = stack.arrange(&[(40.0, 32.0)], 5.0, 7.0);
        assert_eq!(bounds(rects[0]), (5.0, 7.0, 45.0, 39.0));
    }

    #[test]
    fn arrange_should_end_where_desired_size_says_on_the_stack_axis() {
        let sizes = [(40.0, 32.0), (60.0, 48.0), (20.0, 24.0)];
        let horizontal = Stack::new(Orientation::Horizontal).with_spacing(8.0);
        let rects = horizontal.arrange(&sizes, 10.0, 20.0);
        assert_eq!(rects.last().unwrap().right, 10.0 + horizontal.desired_size(&sizes).0);

        let vertical = Stack::new(Orientation::Vertical).with_spacing(4.0);
        let rects = vertical.arrange(&sizes, 10.0, 20.0);
        assert_eq!(rects.last().unwrap().bottom, 20.0 + vertical.desired_size(&sizes).1);
    }

    #[test]
    fn desired_size_should_also_accept_negative_spacing() {
        let stack = Stack::new(Orientation::Horizontal).with_spacing(-4.0);
        assert_eq!(stack.desired_size(&[(40.0, 32.0), (60.0, 32.0)]), (96.0, 32.0));
    }

    #[test]
    fn an_empty_stack_should_have_no_size_and_no_rects() {
        for orientation in [Orientation::Horizontal, Orientation::Vertical] {
            let stack = Stack::new(orientation).with_spacing(8.0);
            assert_eq!(stack.desired_size(&[]), (0.0, 0.0));
            assert!(stack.arrange(&[], 0.0, 0.0).is_empty());
        }
    }

    #[test]
    fn negative_spacing_should_overlap_children() {
        let stack = Stack::new(Orientation::Horizontal).with_spacing(-4.0);
        let rects = stack.arrange(&[(40.0, 32.0), (60.0, 32.0)], 0.0, 0.0);
        assert_eq!(bounds(rects[1]), (36.0, 0.0, 96.0, 32.0));
    }

    #[test]
    fn default_spacing_should_be_zero() {
        let stack = Stack::new(Orientation::Horizontal);
        let rects = stack.arrange(&[(40.0, 32.0), (60.0, 32.0)], 0.0, 0.0);
        assert_eq!(bounds(rects[1]), (40.0, 0.0, 100.0, 32.0));
    }
}
