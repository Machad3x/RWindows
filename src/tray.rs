//! System tray without Explorer: a hidden `Shell_TrayWnd` window receives the
//! `WM_COPYDATA` that `Shell_NotifyIcon` broadcasts to the tray host.
//!
//! The payload is the *platform-neutral* NOTIFYICONDATA: handles are 32-bit
//! even on x64. Casting it to windows-rs' x64 `NOTIFYICONDATAW` would shift
//! every field, so the offsets below are named and tested instead.
//!
//! Running this window while Explorer is alive competes with the real tray.
//! It is meant for a VM where Explorer is not the shell.

use std::ptr;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetWindowThreadProcessId, HWND_BROADCAST, PostMessageW, RegisterClassW,
    RegisterWindowMessageW,
    WM_COPYDATA, WM_CONTEXTMENU, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
};
use windows::core::{Result, w};

/// Class name Explorer's tray uses; apps deliver icons to the first match.
pub const TRAY_CLASS: windows::core::PCWSTR = w!("Shell_TrayWnd");
/// `WM_COPYDATA.dwData` value used by `Shell_NotifyIcon`.
pub const COPYDATA_NOTIFY: usize = 1;
/// Minimum payload with a GUID slot.
pub const MIN_PAYLOAD: usize = 960;

// Byte offsets in the platform-neutral NOTIFYICONDATA.
const OFF_OP: usize = 4;
const OFF_HWND: usize = 12;
const OFF_ID: usize = 16;
const OFF_FLAGS: usize = 20;
const OFF_CALLBACK: usize = 24;
const OFF_ICON: usize = 28;
const OFF_TIP: usize = 32;
const OFF_TIP_END: usize = 288;
const OFF_STATE: usize = 288;
const OFF_STATE_MASK: usize = 292;
const OFF_VERSION: usize = 808;
const OFF_GUID: usize = 944;

const NIF_MESSAGE: u32 = 0x1;
const NIF_ICON: u32 = 0x2;
const NIF_TIP: u32 = 0x4;
const NIF_STATE: u32 = 0x8;
const NIF_GUID: u32 = 0x20;
const NIS_HIDDEN: u32 = 0x1;

/// `Shell_NotifyIcon` operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    /// NIM_ADD.
    Add,
    /// NIM_MODIFY.
    Modify,
    /// NIM_DELETE.
    Delete,
    /// NIM_SETVERSION.
    Version,
}

/// The fields we keep from one notification icon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconData {
    /// Window that receives click messages.
    pub hwnd: usize,
    /// Application icon id.
    pub id: u32,
    /// Callback message number.
    pub callback: u32,
    /// Tooltip text.
    pub tip: String,
    /// Whether the app hid the icon.
    pub hidden: bool,
    /// Shell notification version (affects click message layout).
    pub version: u32,
    /// GUID key when NIF_GUID is set.
    pub guid: [u8; 16],
    /// Raw 32-bit HICON from the payload, zero when NIF_ICON is absent.
    pub hicon: usize,
}

fn u32_at(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

/// Parses a `WM_COPYDATA` payload, or `None` when it is too short.
pub fn parse(data: &[u8]) -> Option<(Operation, IconData)> {
    if data.len() < MIN_PAYLOAD {
        return None;
    }
    // Sign-extend the 32-bit handle, as the platform-neutral format requires.
    let hwnd = u32_at(data, OFF_HWND) as i32 as isize as usize;
    let flags = u32_at(data, OFF_FLAGS);
    let mut tip_units = Vec::new();
    if flags & NIF_TIP != 0 {
        for pair in data[OFF_TIP..OFF_TIP_END].as_chunks::<2>().0 {
            let unit = u16::from_le_bytes([pair[0], pair[1]]);
            if unit == 0 { break }
            tip_units.push(unit);
        }
    }
    let hidden = flags & NIF_STATE != 0
        && u32_at(data, OFF_STATE_MASK) & NIS_HIDDEN != 0
        && u32_at(data, OFF_STATE) & NIS_HIDDEN != 0;
    let guid = if flags & NIF_GUID != 0 {
        data[OFF_GUID..OFF_GUID + 16].try_into().ok()?
    } else {
        [0; 16]
    };
    let icon = IconData {
        hwnd,
        id: u32_at(data, OFF_ID),
        callback: if flags & NIF_MESSAGE != 0 { u32_at(data, OFF_CALLBACK) } else { 0 },
        tip: String::from_utf16_lossy(&tip_units),
        hidden,
        version: u32_at(data, OFF_VERSION),
        guid,
        hicon: if flags & NIF_ICON != 0 {
            u32_at(data, OFF_ICON) as i32 as isize as usize
        } else {
            0
        },
    };
    let operation = match u32_at(data, OFF_OP) {
        0 => Operation::Add,
        1 => Operation::Modify,
        2 => Operation::Delete,
        4 => Operation::Version,
        _ => return None,
    };
    Some((operation, icon))
}

/// One icon plus the shared HICON we copied from the app.
struct Entry {
    data: IconData,
    icon: windows::Win32::UI::WindowsAndMessaging::HICON,
}

/// Owns the hidden tray host window and the icons delivered to it.
pub struct Tray {
    hwnd: HWND,
    icons: Vec<Entry>,
}

impl Tray {
    /// The host window handle.
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Creates the `Shell_TrayWnd` host and installs it as the thread-local
    /// tray that [`handle_message`] updates. Call on the window's thread.
    pub fn install(hinstance: windows::Win32::Foundation::HINSTANCE) -> Result<HWND> {
        unsafe {
            let _ = RegisterClassW(&windows::Win32::UI::WindowsAndMessaging::WNDCLASSW {
                hInstance: hinstance,
                lpszClassName: TRAY_CLASS,
                lpfnWndProc: Some(tray_proc),
                ..Default::default()
            });
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST, TRAY_CLASS,
                w!(""), windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(0),
                0, 0, 0, 0, None, None, Some(hinstance), None,
            )?;
            TRAY.with(|cell| *cell.borrow_mut() = Some(Self { hwnd, icons: Vec::new() }));
            Ok(hwnd)
        }
    }

    /// Applies one parsed notification to the icon list.
    pub fn apply(&mut self, operation: Operation, incoming: IconData) {
        let position = self.icons.iter().position(|entry| {
            entry.data.hwnd == incoming.hwnd
                && entry.data.id == incoming.id
                && entry.data.guid == incoming.guid
        });
        match (operation, position) {
            (Operation::Delete, Some(index)) => {
                let entry = self.icons.remove(index);
                if !entry.icon.0.is_null() {
                    unsafe { _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(entry.icon) };
                }
            }
            (Operation::Version, Some(index)) => self.icons[index].data.version = incoming.version,
            (Operation::Add | Operation::Modify, _) => {
                let index = match position {
                    Some(index) => index,
                    None if operation == Operation::Modify => return,
                    None => {
                        self.icons.push(Entry {
                            data: incoming.clone(),
                            icon: windows::Win32::UI::WindowsAndMessaging::HICON(ptr::null_mut()),
                        });
                        self.icons.len() - 1
                    }
                };
                if incoming.hicon != 0 {
                    let copied = unsafe {
                        windows::Win32::UI::WindowsAndMessaging::CopyIcon(
                            windows::Win32::UI::WindowsAndMessaging::HICON(incoming.hicon as isize as *mut core::ffi::c_void),
                        ).unwrap_or_default()
                    };
                    if !copied.0.is_null() {
                        if !self.icons[index].icon.0.is_null() {
                            unsafe { _ = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(self.icons[index].icon) };
                        }
                        self.icons[index].icon = copied;
                    }
                }
                if incoming.hidden {
                    self.icons[index].data.hidden = true;
                }
                self.icons[index].data.tip = incoming.tip;
                self.icons[index].data.callback = incoming.callback;
            }
            _ => {}
        }
    }

    /// Number of icons currently tracked.
    pub fn len(&self) -> usize {
        self.icons.len()
    }

    /// Whether no icons are tracked.
    pub fn is_empty(&self) -> bool {
        self.icons.is_empty()
    }

    /// Replays the messages Explorer would send for a click, so the app shows
    /// its own window or menu.
    pub fn click(&self, index: usize, left: bool) -> Result<()> {
        let Some(entry) = self.icons.get(index) else { return Ok(()) };
        if entry.data.callback == 0 {
            return Ok(());
        }
        let hwnd = HWND(entry.data.hwnd as isize as *mut core::ffi::c_void);
        unsafe {
            let mut pid = 0u32;
            _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
            _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid);
            let mut point = windows::Win32::Foundation::POINT::default();
            _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point);
            let post = |event: u32| {
                if entry.data.version >= 4 {
                    let x = (point.x as u32) & 0xffff;
                    let y = (point.y as u32) << 16;
                    let id = (entry.data.id & 0xffff) << 16;
                    _ = PostMessageW(Some(hwnd), entry.data.callback,
                        WPARAM((x | y) as usize), LPARAM((event | id) as isize));
                } else {
                    _ = PostMessageW(Some(hwnd), entry.data.callback,
                        WPARAM(entry.data.id as usize), LPARAM(event as isize));
                }
            };
            // shortcut: no double-click or hover state yet; some apps need
            // WM_LBUTTONDBLCLK to open, add it with GetDoubleClickTime if one
            // turns out to depend on it.
            let _ = GetDoubleClickTime();
            if left {
                post(WM_LBUTTONDOWN);
                post(WM_LBUTTONUP);
                if entry.data.version >= 4 { post(0x400); } // NIN_SELECT
            } else {
                post(WM_RBUTTONDOWN);
                post(WM_RBUTTONUP);
                if entry.data.version >= 4 { post(WM_CONTEXTMENU); }
            }
        }
        Ok(())
    }
}

thread_local! {
    static TRAY: std::cell::RefCell<Option<Tray>> = const { std::cell::RefCell::new(None) };
}

/// Number of icons in the installed tray.
pub fn len() -> usize {
    TRAY.with(|cell| cell.borrow().as_ref().map_or(0, |tray| tray.icons.len()))
}

/// Whether the installed tray has no icons.
pub fn is_empty() -> bool {
    len() == 0
}

/// Replays a click for icon `index` in the installed tray.
pub fn click(index: usize, left: bool) -> Result<()> {
    TRAY.with(|cell| {
        let Ok(guard) = cell.try_borrow() else { return Ok(()) };
        match guard.as_ref() {
            Some(tray) => tray.click(index, left),
            None => Ok(()),
        }
    })
}

/// Handles `WM_COPYDATA`; returns `true` when the message was consumed.
pub fn handle_message(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    if msg != WM_COPYDATA {
        return None;
    }
    unsafe {
        let copy = &*(lp.0 as *const COPYDATASTRUCT);
        if copy.dwData == COPYDATA_NOTIFY && copy.cbData as usize >= MIN_PAYLOAD {
            let data = std::slice::from_raw_parts(copy.lpData.cast::<u8>(), copy.cbData as usize);
            if let Some((operation, icon)) = parse(data) {
                TRAY.with(|cell| {
                    if let Ok(mut guard) = cell.try_borrow_mut()
                        && let Some(tray) = guard.as_mut()
                    {
                        tray.apply(operation, icon);
                    }
                });
            }
            return Some(LRESULT(1));
        }
    }
    let _ = (hwnd, wp);
    None
}

unsafe extern "system" fn tray_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if let Some(result) = handle_message(hwnd, msg, wp, lp) {
        return result;
    }
    unsafe { windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(hwnd, msg, wp, lp) }
}

/// Broadcasts `TaskbarCreated` so apps re-add their icons after the host starts.
pub fn announce() -> Result<()> {
    unsafe {
        let message = RegisterWindowMessageW(w!("TaskbarCreated"));
        if message == 0 {
            return Err(windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL));
        }
        PostMessageW(Some(HWND_BROADCAST), message, WPARAM(0), LPARAM(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(operation: u32, flags: u32, id: u32, callback: u32, hidden: bool, version: u32) -> Vec<u8> {
        let mut data = vec![0u8; MIN_PAYLOAD];
        for (offset, value) in [(OFF_OP, operation), (OFF_ID, id), (OFF_FLAGS, flags),
            (OFF_CALLBACK, callback), (OFF_VERSION, version)] {
            data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        if flags & NIF_STATE != 0 {
            data[OFF_STATE_MASK..OFF_STATE_MASK + 4].copy_from_slice(&NIS_HIDDEN.to_le_bytes());
            if hidden {
                data[OFF_STATE..OFF_STATE + 4].copy_from_slice(&NIS_HIDDEN.to_le_bytes());
            }
        }
        data
    }

    #[test]
    fn parse_should_read_a_version_four_icon() {
        let mut data = payload(0, NIF_MESSAGE | NIF_TIP, 7, 0x500, false, 4);
        for (index, unit) in "Área de trabalho".encode_utf16().enumerate() {
            data[OFF_TIP + index * 2..OFF_TIP + index * 2 + 2].copy_from_slice(&unit.to_le_bytes());
        }
        let (operation, icon) = parse(&data).unwrap();
        assert_eq!(operation, Operation::Add);
        assert_eq!((icon.id, icon.callback, icon.version), (7, 0x500, 4));
        assert_eq!(icon.tip, "Área de trabalho");
        assert!(!icon.hidden);
    }

    #[test]
    fn parse_should_reject_short_payloads() {
        assert!(parse(&[0u8; 100]).is_none());
    }

    #[test]
    fn parse_should_map_all_operations() {
        assert_eq!(parse(&payload(0, 0, 0, 0, false, 0)).unwrap().0, Operation::Add);
        assert_eq!(parse(&payload(1, 0, 0, 0, false, 0)).unwrap().0, Operation::Modify);
        assert_eq!(parse(&payload(2, 0, 0, 0, false, 0)).unwrap().0, Operation::Delete);
        assert_eq!(parse(&payload(4, 0, 0, 0, false, 0)).unwrap().0, Operation::Version);
        assert!(parse(&payload(9, 0, 0, 0, false, 0)).is_none());
    }

    #[test]
    fn hidden_state_should_only_apply_when_masked() {
        let (_, hidden) = parse(&payload(0, NIF_STATE, 1, 0, true, 0)).unwrap();
        let (_, shown) = parse(&payload(0, NIF_STATE, 1, 0, false, 0)).unwrap();
        assert!(hidden.hidden);
        assert!(!shown.hidden);
    }

    #[test]
    fn add_modify_and_delete_should_track_one_icon() {
        let mut tray = Tray { hwnd: HWND(ptr::null_mut()), icons: Vec::new() };
        let (_, icon) = parse(&payload(0, NIF_MESSAGE, 5, 0x600, false, 4)).unwrap();
        tray.apply(Operation::Add, icon.clone());
        assert_eq!(tray.len(), 1);
        tray.apply(Operation::Modify, IconData { tip: "new".into(), ..icon.clone() });
        assert_eq!(tray.len(), 1);
        tray.apply(Operation::Delete, icon);
        assert!(tray.is_empty());
    }
}
