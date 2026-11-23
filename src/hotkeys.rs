//! Global Win-key shortcuts through a low-level keyboard hook.
//!
//! `RegisterHotKey` cannot take Win+E/D/R while Explorer owns them, so the hook
//! sees the keys first and posts [`WM_SHORTCUT`] to a window of our choice. The
//! heavy work (opening Explorer, showing the desktop) runs on that window's
//! thread, never inside the hook, which must return quickly.

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VK_D, VK_E, VK_LWIN, VK_RWIN, VIRTUAL_KEY, keybd_event,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, HHOOK, KBDLLHOOKSTRUCT, PostMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
    WH_KEYBOARD_LL, WM_APP, WM_KEYDOWN, WM_SYSKEYDOWN,
};
use windows::core::Result;

/// Message posted to the hook's target window. `wParam` is an [`Action`] value.
pub const WM_SHORTCUT: u32 = WM_APP + 0x100;

/// A Windows-key shortcut the shell should handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Win+E: open File Explorer.
    Explorer,
    /// Win+D: show the desktop / restore windows.
    ShowDesktop,
}

impl Action {
    /// Decodes the `wParam` of a [`WM_SHORTCUT`] message.
    pub fn from_usize(value: usize) -> Option<Self> {
        match value {
            0 => Some(Self::Explorer),
            1 => Some(Self::ShowDesktop),
            _ => None,
        }
    }

    /// Maps a virtual key pressed while Win is held to an action.
    pub fn from_vk(vk: VIRTUAL_KEY) -> Option<Self> {
        match vk {
            VK_E => Some(Self::Explorer),
            VK_D => Some(Self::ShowDesktop),
            _ => None,
        }
    }
}

#[derive(Default)]
struct State {
    win_down: bool,
    target: Option<HWND>,
}

thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State { win_down: false, target: None }) };
}

/// Owns the global hook. Dropping it unhooks the keyboard.
pub struct KeyboardHook {
    hook: HHOOK,
}

impl KeyboardHook {
    /// Installs the hook. Shortcut messages go to `target`.
    pub fn new(target: HWND) -> Result<Self> {
        STATE.with(|cell| {
            let mut state = cell.borrow_mut();
            state.target = Some(target);
            state.win_down = false;
        });
        // Low-level hooks run on the installing thread; hMod is NULL when the
        // procedure lives in this executable.
        let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0)? };
        Ok(Self { hook })
    }
}

impl Drop for KeyboardHook {
    fn drop(&mut self) {
        unsafe { _ = UnhookWindowsHookEx(self.hook) };
        STATE.with(|cell| cell.borrow_mut().target = None);
    }
}

unsafe extern "system" fn hook_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code >= 0 {
        let info = unsafe { &*(lp.0 as *const KBDLLHOOKSTRUCT) };
        let down = wp.0 as u32 == WM_KEYDOWN || wp.0 as u32 == WM_SYSKEYDOWN;
        // Our own marker key must not re-trigger the hook.
        if info.vkCode == 0xE8 {
            return unsafe { CallNextHookEx(None, code, wp, lp) };
        }
        let handled = STATE.with(|cell| {
            let mut state = cell.borrow_mut();
            if info.vkCode == VK_LWIN.0 as u32 || info.vkCode == VK_RWIN.0 as u32 {
                state.win_down = down;
                return false;
            }
            if !(down && state.win_down) {
                return false;
            }
            let Some(action) = Action::from_vk(VIRTUAL_KEY(info.vkCode as u16)) else { return false };
            let Some(target) = state.target else { return false };
            unsafe {
                _ = PostMessageW(Some(target), WM_SHORTCUT, WPARAM(action as usize), LPARAM(0));
                // shortcut: 0xE8 is unassigned. A dummy press makes Windows skip
                // opening Start when Win is released; replace if Windows reuses it.
                keybd_event(0xE8, 0, KEYBD_EVENT_FLAGS(0), 0);
                keybd_event(0xE8, 0, KEYEVENTF_KEYUP, 0);
            }
            true
        });
        if handled {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_mapped_keys_become_actions() {
        assert_eq!(Action::from_vk(VK_E), Some(Action::Explorer));
        assert_eq!(Action::from_vk(VK_D), Some(Action::ShowDesktop));
        assert_eq!(Action::from_vk(VK_LWIN), None);
    }

}
