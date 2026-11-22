//! UI Automation for custom-drawn controls (screen readers, Narrator, test tools).
//!
//! A window exposes one fragment root ([`root`]) from `WM_GETOBJECT`
//! (`UiaRootObjectId`); each control is a fragment child. Providers read a
//! shared [`Model`] the app refreshes after every change, so a UIA call never
//! competes with the app's own borrow. `Invoke` and `SetFocus` post
//! [`WM_UIA_INVOKE`] / [`WM_UIA_FOCUS`] (index in `wParam`): `Invoke` must return
//! immediately (Learn: `IInvokeProvider::Invoke`).
//!
//! Threading: `ProviderOptions_UseComThreading`; the UI thread must be an STA
//! (`OleInitialize` / `CoInitializeEx(COINIT_APARTMENTTHREADED)`).

#![expect(non_upper_case_globals, reason = "matching Win32 constants keeps their SDK names")]

use std::cell::RefCell;
use std::rc::Rc;

use windows::Win32::Foundation::{E_OUTOFMEMORY, HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayPutElement};
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};
use windows::core::{Error, HRESULT, IUnknown, IUnknownImpl, Interface, Result, implement};

/// `UIA_E_ELEMENTNOTENABLED` as an `HRESULT` (the crate exposes it as `u32`).
const NOT_ENABLED: HRESULT = HRESULT(UIA_E_ELEMENTNOTENABLED as i32);
/// `UIA_E_ELEMENTNOTAVAILABLE` as an `HRESULT`.
const NOT_AVAILABLE: HRESULT = HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32);

/// Posted by `IInvokeProvider::Invoke` and `IToggleProvider::Toggle`; `wParam` = control index.
pub const WM_UIA_INVOKE: u32 = WM_APP + 0x55;
/// Posted by `IRawElementProviderFragment::SetFocus`; `wParam` = control index.
pub const WM_UIA_FOCUS: u32 = WM_APP + 0x56;

/// One control as UI Automation sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// `Name` (the label).
    pub name: String,
    /// Bounds in client DIPs.
    pub rect: D2D_RECT_F,
    /// `IsEnabled`.
    pub enabled: bool,
    /// `ToggleState` (On, Off, Indeterminate) of a toggle control (Toggle pattern
    /// instead of Invoke); `None` for a plain button.
    pub toggle: Option<ToggleState>,
    /// `ControlType`: `UIA_ButtonControlTypeId` (Button, ToggleButton, ToggleSwitch) or
    /// `UIA_CheckBoxControlTypeId` (`CheckBoxAutomationPeer`).
    pub control_type: UIA_CONTROLTYPE_ID,
}

/// What the providers expose; the app keeps it current.
#[derive(Clone, Debug, Default)]
pub struct Model {
    /// Window hosting the controls.
    pub hwnd: HWND,
    /// Pixels per DIP.
    pub scale: f32,
    /// Controls in navigation (tab) order.
    pub items: Vec<Item>,
    /// Index of the control with keyboard focus.
    pub focused: Option<usize>,
}

/// Shared model handle.
pub type Shared = Rc<RefCell<Model>>;

/// The fragment root to return from `WM_GETOBJECT` via `UiaReturnRawElementProvider`.
pub fn root(model: &Shared) -> IRawElementProviderSimple {
    Root { model: model.clone() }.into()
}

/// Provider of control `index` (for raising events on it).
pub fn item(model: &Shared, index: usize) -> IRawElementProviderSimple {
    ItemProvider { model: model.clone(), index }.into()
}

#[implement(IRawElementProviderSimple, IRawElementProviderFragment, IRawElementProviderFragmentRoot)]
struct Root {
    model: Shared,
}

#[implement(IRawElementProviderSimple, IRawElementProviderFragment, IInvokeProvider, IToggleProvider)]
struct ItemProvider {
    model: Shared,
    index: usize,
}

/// `Err(Error::empty())` is how windows-rs returns S_OK with a null interface.
/// The shim does not write the out pointer, so the caller's zero-initialized slot
/// stays NULL: true for UIA core, NDR stubs (the UseComThreading path) and
/// windows-rs callers. A raw caller passing an uninitialized slot would misread it.
fn null<T>() -> Result<T> {
    Err(Error::empty())
}

const OPTIONS: ProviderOptions = ProviderOptions(ProviderOptions_ServerSideProvider.0 | ProviderOptions_UseComThreading.0);

fn fragment_of(model: &Shared, index: usize) -> Result<IRawElementProviderFragment> {
    Ok(ItemProvider { model: model.clone(), index }.into())
}

fn root_fragment(model: &Shared) -> Result<IRawElementProviderFragmentRoot> {
    Ok(Root { model: model.clone() }.into())
}

/// Screen position (pixels) of the client area's top-left corner.
fn client_origin(hwnd: HWND) -> POINT {
    let mut p = POINT::default();
    unsafe { _ = ClientToScreen(hwnd, &mut p) };
    p
}

impl IRawElementProviderSimple_Impl for Root_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(OPTIONS)
    }

    fn GetPatternProvider(&self, _id: UIA_PATTERN_ID) -> Result<IUnknown> {
        null()
    }

    fn GetPropertyValue(&self, _id: UIA_PROPERTY_ID) -> Result<VARIANT> {
        // The window's host provider supplies Name, ControlType, bounds, etc.
        Ok(VARIANT::default())
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        unsafe { UiaHostProviderFromHwnd(self.model.borrow().hwnd) }
    }
}

impl IRawElementProviderFragment_Impl for Root_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let n = self.model.borrow().items.len();
        match direction {
            NavigateDirection_FirstChild if n > 0 => fragment_of(&self.model, 0),
            NavigateDirection_LastChild if n > 0 => fragment_of(&self.model, n - 1),
            // Parent and siblings of a window-hosted root come from the host provider.
            _ => null(),
        }
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        // NULL for a root hosted in a window (Learn: GetRuntimeId).
        Ok(std::ptr::null_mut())
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        Ok(UiaRect::default())
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        Ok(())
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(self.to_object().to_interface())
    }
}

impl IRawElementProviderFragmentRoot_Impl for Root_Impl {
    fn ElementProviderFromPoint(&self, x: f64, y: f64) -> Result<IRawElementProviderFragment> {
        let m = self.model.borrow();
        let o = client_origin(m.hwnd);
        let (cx, cy) = (((x - f64::from(o.x)) as f32) / m.scale, ((y - f64::from(o.y)) as f32) / m.scale);
        let hit = m.items.iter().position(|i| cx >= i.rect.left && cx < i.rect.right && cy >= i.rect.top && cy < i.rect.bottom);
        drop(m);
        hit.map_or_else(null, |i| fragment_of(&self.model, i))
    }

    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        let focused = self.model.borrow().focused;
        focused.map_or_else(null, |i| fragment_of(&self.model, i))
    }
}

impl ItemProvider_Impl {
    /// The control this provider stands for, or `UIA_E_ELEMENTNOTAVAILABLE` once it is gone.
    fn info(&self) -> Result<Item> {
        self.model.borrow().items.get(self.index).cloned().ok_or_else(|| Error::from(NOT_AVAILABLE))
    }

    fn post(&self, msg: u32) -> Result<()> {
        let hwnd = self.model.borrow().hwnd;
        unsafe { PostMessageW(Some(hwnd), msg, WPARAM(self.index), LPARAM(0)) }
    }
}

impl IRawElementProviderSimple_Impl for ItemProvider_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(OPTIONS)
    }

    fn GetPatternProvider(&self, id: UIA_PATTERN_ID) -> Result<IUnknown> {
        // ToggleButtonAutomationPeer exposes Toggle only; ButtonAutomationPeer, Invoke.
        let pattern = if self.info()?.toggle.is_some() { UIA_TogglePatternId } else { UIA_InvokePatternId };
        if id == pattern { Ok(self.to_object().to_interface()) } else { null() }
    }

    fn GetPropertyValue(&self, id: UIA_PROPERTY_ID) -> Result<VARIANT> {
        let item = self.info()?;
        let focused = self.model.borrow().focused == Some(self.index);
        Ok(match id {
            UIA_ControlTypePropertyId => VARIANT::from(item.control_type.0),
            UIA_NamePropertyId => VARIANT::from(item.name.as_str()),
            // Unique among peers (Button control type requirement).
            UIA_AutomationIdPropertyId => VARIANT::from(format!("button-{}", self.index).as_str()),
            UIA_IsEnabledPropertyId => VARIANT::from(item.enabled),
            UIA_IsKeyboardFocusablePropertyId => VARIANT::from(item.enabled),
            UIA_HasKeyboardFocusPropertyId => VARIANT::from(focused),
            UIA_IsControlElementPropertyId | UIA_IsContentElementPropertyId => VARIANT::from(true),
            UIA_FrameworkIdPropertyId => VARIANT::from("fluent-win"),
            // Empty = not supported here; UIA fills defaults (e.g. LocalizedControlType).
            _ => VARIANT::default(),
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        // Fragment children are not hosted by a window.
        null()
    }
}

impl IRawElementProviderFragment_Impl for ItemProvider_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        self.info()?;
        let n = self.model.borrow().items.len();
        match direction {
            NavigateDirection_Parent => root_fragment(&self.model)?.cast(),
            NavigateDirection_NextSibling if self.index + 1 < n => fragment_of(&self.model, self.index + 1),
            NavigateDirection_PreviousSibling if self.index > 0 => fragment_of(&self.model, self.index - 1),
            _ => null(),
        }
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        self.info()?;
        // {UiaAppendRuntimeId, unique value}; UIA prefixes the host window's id.
        let parts = [UiaAppendRuntimeId as i32, self.index as i32 + 1];
        unsafe {
            let sa = SafeArrayCreateVector(VT_I4, 0, parts.len() as u32);
            if sa.is_null() {
                return Err(Error::from(E_OUTOFMEMORY));
            }
            for (i, v) in (0i32..).zip(parts) {
                SafeArrayPutElement(sa, &i, (&raw const v).cast())?;
            }
            Ok(sa)
        }
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        let item = self.info()?;
        let (hwnd, k) = {
            let m = self.model.borrow();
            (m.hwnd, f64::from(m.scale))
        };
        let o = client_origin(hwnd);
        let r = item.rect;
        // Screen pixels, unclipped (Learn: BoundingRectangle).
        Ok(UiaRect {
            left: f64::from(o.x) + f64::from(r.left) * k,
            top: f64::from(o.y) + f64::from(r.top) * k,
            width: f64::from(r.right - r.left) * k,
            height: f64::from(r.bottom - r.top) * k,
        })
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        if !self.info()?.enabled {
            return Err(Error::from(NOT_ENABLED));
        }
        // UIA already focused the window; the app moves its own focus on this message.
        self.post(WM_UIA_FOCUS)
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        root_fragment(&self.model)
    }
}

impl IInvokeProvider_Impl for ItemProvider_Impl {
    fn Invoke(&self) -> Result<()> {
        if !self.info()?.enabled {
            return Err(Error::from(NOT_ENABLED));
        }
        // Must return immediately: the click runs when the app handles the message.
        self.post(WM_UIA_INVOKE)
    }
}

impl IToggleProvider_Impl for ItemProvider_Impl {
    fn Toggle(&self) -> Result<()> {
        if !self.info()?.enabled {
            return Err(Error::from(NOT_ENABLED));
        }
        // Like Invoke: the app runs the click (which flips IsChecked) on the message,
        // then raises the ToggleState property-changed event.
        self.post(WM_UIA_INVOKE)
    }

    fn ToggleState(&self) -> Result<ToggleState> {
        Ok(self.info()?.toggle.unwrap_or(ToggleState_Off))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::rect;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, MSG, PM_REMOVE, PeekMessageW, WINDOW_EX_STYLE, WS_POPUP,
    };
    use windows::core::w;

    /// Hidden popup at screen (0, 0): client DIPs map to screen pixels by `scale` alone.
    fn model(scale: f32) -> Shared {
        let hwnd = unsafe {
            CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 0, 0, 400, 100, None, None, None, None)
                .unwrap()
        };
        Rc::new(RefCell::new(Model {
            hwnd,
            scale,
            items: vec![
                Item { name: "Configurações".into(), rect: rect(10.0, 10.0, 110.0, 42.0), enabled: true, toggle: None, control_type: UIA_ButtonControlTypeId },
                Item { name: "Salvar".into(), rect: rect(120.0, 10.0, 180.0, 42.0), enabled: true, toggle: None, control_type: UIA_ButtonControlTypeId },
                Item { name: "Desabilitado".into(), rect: rect(190.0, 10.0, 290.0, 42.0), enabled: false, toggle: None, control_type: UIA_ButtonControlTypeId },
            ],
            focused: Some(1),
        }))
    }

    fn fragment(p: &IRawElementProviderSimple) -> IRawElementProviderFragment {
        p.cast().unwrap()
    }

    fn name_of(f: &IRawElementProviderFragment) -> String {
        let simple: IRawElementProviderSimple = f.cast().unwrap();
        bstr(&unsafe { simple.GetPropertyValue(UIA_NamePropertyId).unwrap() })
    }

    /// Reads a VT_BSTR variant (BSTR conversion needs an extra crate feature).
    fn bstr(v: &VARIANT) -> String {
        use windows::Win32::System::Variant::VT_BSTR;
        unsafe {
            assert_eq!(v.Anonymous.Anonymous.vt, VT_BSTR);
            v.Anonymous.Anonymous.Anonymous.bstrVal.to_string()
        }
    }

    fn prop(m: &Shared, i: usize, id: UIA_PROPERTY_ID) -> VARIANT {
        unsafe { item(m, i).GetPropertyValue(id).unwrap() }
    }

    #[test]
    fn root_should_navigate_children_in_order() {
        let m = model(1.0);
        let root = fragment(&root(&m));
        let first = unsafe { root.Navigate(NavigateDirection_FirstChild).unwrap() };
        let second = unsafe { first.Navigate(NavigateDirection_NextSibling).unwrap() };
        let last = unsafe { root.Navigate(NavigateDirection_LastChild).unwrap() };
        assert_eq!((name_of(&first), name_of(&second), name_of(&last)), ("Configurações".into(), "Salvar".into(), "Desabilitado".into()));
    }

    #[test]
    fn edges_of_the_sibling_list_should_be_null() {
        let m = model(1.0);
        let first = fragment(&item(&m, 0));
        let last = fragment(&item(&m, 2));
        let before = unsafe { first.Navigate(NavigateDirection_PreviousSibling) };
        let after = unsafe { last.Navigate(NavigateDirection_NextSibling) };
        // windows-rs maps a null out pointer to Err with S_OK-like empty code.
        assert!(before.is_err() && after.is_err());
    }

    #[test]
    fn child_parent_and_fragment_root_should_be_the_root() {
        let m = model(1.0);
        let child = fragment(&item(&m, 1));
        let parent = unsafe { child.Navigate(NavigateDirection_Parent).unwrap() };
        let root_of = unsafe { child.FragmentRoot().unwrap() };
        // UIA identifies a window-hosted root by its host (runtime id NULL), not by COM identity.
        let hosted = |f: &IRawElementProviderFragment| {
            let simple: IRawElementProviderSimple = f.cast().unwrap();
            unsafe { simple.HostRawElementProvider() }.is_ok()
        };
        let first = |f: &IRawElementProviderFragment| name_of(&unsafe { f.Navigate(NavigateDirection_FirstChild).unwrap() });
        let root_of: IRawElementProviderFragment = root_of.cast().unwrap();
        assert_eq!((hosted(&parent), hosted(&root_of), first(&parent), first(&root_of)),
            (true, true, "Configurações".to_owned(), "Configurações".to_owned()));
    }

    #[test]
    fn item_should_be_a_named_button() {
        let m = model(1.0);
        let control_type = i32::try_from(&prop(&m, 1, UIA_ControlTypePropertyId)).unwrap();
        let name = bstr(&prop(&m, 1, UIA_NamePropertyId));
        assert_eq!((control_type, name.as_str()), (UIA_ButtonControlTypeId.0, "Salvar"));
    }

    #[test]
    fn control_type_should_come_from_the_model() {
        let m = model(1.0);
        m.borrow_mut().items[0].control_type = UIA_CheckBoxControlTypeId;
        let control_type = i32::try_from(&prop(&m, 0, UIA_ControlTypePropertyId)).unwrap();
        assert_eq!(control_type, UIA_CheckBoxControlTypeId.0);
    }

    #[test]
    fn automation_ids_should_be_unique() {
        let m = model(1.0);
        let ids: Vec<String> = (0..3).map(|i| bstr(&prop(&m, i, UIA_AutomationIdPropertyId))).collect();
        assert!(ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2]);
    }

    #[test]
    fn enabled_focusable_and_focus_should_follow_the_model() {
        let m = model(1.0);
        let flag = |i, id| bool::try_from(&prop(&m, i, id)).unwrap();
        assert_eq!(
            (flag(1, UIA_IsEnabledPropertyId), flag(2, UIA_IsEnabledPropertyId), flag(2, UIA_IsKeyboardFocusablePropertyId),
             flag(1, UIA_HasKeyboardFocusPropertyId), flag(0, UIA_HasKeyboardFocusPropertyId)),
            (true, false, false, true, false)
        );
    }

    #[test]
    fn item_should_be_control_and_content_element() {
        let m = model(1.0);
        let flag = |id| bool::try_from(&prop(&m, 0, id)).unwrap();
        assert!(flag(UIA_IsControlElementPropertyId) && flag(UIA_IsContentElementPropertyId));
    }

    #[test]
    fn bounding_rectangle_should_be_in_screen_pixels() {
        let m = model(1.5);
        let r = unsafe { fragment(&item(&m, 1)).BoundingRectangle().unwrap() };
        assert_eq!((r.left, r.top, r.width, r.height), (180.0, 15.0, 90.0, 48.0));
    }

    #[test]
    fn element_from_point_should_hit_the_button_under_it() {
        let m = model(1.0);
        let root: IRawElementProviderFragmentRoot = root(&m).cast().unwrap();
        let hit = unsafe { root.ElementProviderFromPoint(130.0, 20.0).unwrap() };
        assert_eq!(name_of(&hit), "Salvar");
    }

    #[test]
    fn get_focus_should_return_the_focused_button() {
        let m = model(1.0);
        let root: IRawElementProviderFragmentRoot = root(&m).cast().unwrap();
        let focused = unsafe { root.GetFocus().unwrap() };
        assert_eq!(name_of(&focused), "Salvar");
    }

    #[test]
    fn runtime_ids_should_differ_between_buttons() {
        let m = model(1.0);
        let a = unsafe { fragment(&item(&m, 0)).GetRuntimeId().unwrap() };
        let b = unsafe { fragment(&item(&m, 1)).GetRuntimeId().unwrap() };
        assert!(!a.is_null() && !b.is_null() && read_i32s(a) != read_i32s(b));
    }

    fn read_i32s(sa: *mut SAFEARRAY) -> Vec<i32> {
        use windows::Win32::System::Ole::{SafeArrayDestroy, SafeArrayGetElement, SafeArrayGetUBound};
        unsafe {
            let upper = SafeArrayGetUBound(sa, 1).unwrap();
            let v = (0..=upper).map(|i| {
                let mut x = 0i32;
                SafeArrayGetElement(sa, &i, (&raw mut x).cast()).unwrap();
                x
            }).collect();
            SafeArrayDestroy(sa).unwrap();
            v
        }
    }

    fn take_posted(hwnd: HWND, msg: u32) -> Option<(WPARAM, LPARAM)> {
        let mut m = MSG::default();
        unsafe { PeekMessageW(&mut m, Some(hwnd), msg, msg, PM_REMOVE) }.as_bool().then_some((m.wParam, m.lParam))
    }

    #[test]
    fn invoke_should_post_the_index_and_return_at_once() {
        let m = model(1.0);
        let invoke: IInvokeProvider = unsafe { item(&m, 1).GetPatternProvider(UIA_InvokePatternId).unwrap() }.cast().unwrap();
        unsafe { invoke.Invoke().unwrap() };
        assert_eq!(take_posted(m.borrow().hwnd, WM_UIA_INVOKE).map(|(w, _)| w.0), Some(1));
    }

    #[test]
    fn invoke_on_a_disabled_button_should_fail_with_not_enabled() {
        let m = model(1.0);
        let invoke: IInvokeProvider = unsafe { item(&m, 2).GetPatternProvider(UIA_InvokePatternId).unwrap() }.cast().unwrap();
        assert_eq!(unsafe { invoke.Invoke() }.map_err(|e| e.code()), Err(NOT_ENABLED));
    }

    #[test]
    fn set_focus_should_post_the_index() {
        let m = model(1.0);
        unsafe { fragment(&item(&m, 0)).SetFocus().unwrap() };
        assert_eq!(take_posted(m.borrow().hwnd, WM_UIA_FOCUS).map(|(w, _)| w.0), Some(0));
    }

    #[test]
    fn removed_item_should_report_element_not_available() {
        let m = model(1.0);
        let gone = item(&m, 2);
        m.borrow_mut().items.truncate(1);
        assert_eq!(unsafe { gone.GetPropertyValue(UIA_NamePropertyId) }.map_err(|e| e.code()).err(), Some(NOT_AVAILABLE));
    }

    /// Appends toggle button 3 (`checked`, `enabled`).
    fn with_toggle(m: &Shared, checked: bool, enabled: bool) -> usize {
        let item = Item { name: "Wi-Fi".into(), rect: rect(300.0, 10.0, 360.0, 42.0), enabled, toggle: Some(if checked { ToggleState_On } else { ToggleState_Off }), control_type: UIA_ButtonControlTypeId };
        m.borrow_mut().items.push(item);
        3
    }

    fn toggle_of(m: &Shared, i: usize) -> IToggleProvider {
        unsafe { item(m, i).GetPatternProvider(UIA_TogglePatternId).unwrap() }.cast().unwrap()
    }

    #[test]
    fn toggle_button_should_expose_toggle_and_not_invoke() {
        let m = model(1.0);
        let i = with_toggle(&m, false, true);
        let invoke = unsafe { item(&m, i).GetPatternProvider(UIA_InvokePatternId) };
        let toggle = unsafe { item(&m, i).GetPatternProvider(UIA_TogglePatternId) };
        assert_eq!((invoke.is_err(), toggle.is_ok()), (true, true));
    }

    #[test]
    fn plain_button_should_not_expose_toggle() {
        let m = model(1.0);
        assert!(unsafe { item(&m, 1).GetPatternProvider(UIA_TogglePatternId) }.is_err());
    }

    #[test]
    fn toggle_state_should_follow_the_model() {
        let m = model(1.0);
        let i = with_toggle(&m, true, true);
        let on = unsafe { toggle_of(&m, i).ToggleState().unwrap() };
        m.borrow_mut().items[i].toggle = Some(ToggleState_Off);
        let off = unsafe { toggle_of(&m, i).ToggleState().unwrap() };
        m.borrow_mut().items[i].toggle = Some(ToggleState_Indeterminate);
        let mixed = unsafe { toggle_of(&m, i).ToggleState().unwrap() };
        assert_eq!((on, off, mixed), (ToggleState_On, ToggleState_Off, ToggleState_Indeterminate));
    }

    #[test]
    fn toggle_should_post_the_index() {
        let m = model(1.0);
        let i = with_toggle(&m, false, true);
        unsafe { toggle_of(&m, i).Toggle().unwrap() };
        assert_eq!(take_posted(m.borrow().hwnd, WM_UIA_INVOKE).map(|(w, _)| w.0), Some(i));
    }

    #[test]
    fn toggle_on_a_disabled_button_should_fail_with_not_enabled() {
        let m = model(1.0);
        let i = with_toggle(&m, false, false);
        assert_eq!(unsafe { toggle_of(&m, i).Toggle() }.map_err(|e| e.code()), Err(NOT_ENABLED));
    }

    #[test]
    fn root_should_use_com_threading_and_be_hosted_by_the_window() {
        let m = model(1.0);
        let r = root(&m);
        let options = unsafe { r.ProviderOptions().unwrap() };
        let host = unsafe { r.HostRawElementProvider() };
        assert_eq!((options, host.is_ok()), (ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading, true));
    }
}
