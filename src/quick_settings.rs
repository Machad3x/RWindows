//! Backend for Quick Settings: system volume through `IAudioEndpointVolume`.
//! Everything here is documented Win32 (MSDN); COM must be initialized on the
//! calling thread with [`init_com`] before [`Volume::new`].

use windows::Win32::Devices::Display::{
    IOCTL_VIDEO_QUERY_DISPLAY_BRIGHTNESS, IOCTL_VIDEO_SET_DISPLAY_BRIGHTNESS,
};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, eMultimedia, eRender};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::core::{GUID, HSTRING, Result};

/// CLSID_MMDeviceEnumerator, from the MMDevice API reference on MSDN.
const CLSID_MM_DEVICE_ENUMERATOR: GUID = GUID::from_u128(0xbcde0395_e52f_467c_8e3d_c4579291692e);

/// Initializes COM for the calling thread. `RPC_E_CHANGED_MODE` means another
/// mode was already set on this thread, which is fine for our purposes.
pub fn init_com() {
    unsafe {
        _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

/// Default render endpoint's master volume. Keeps the enumerator, not one
/// endpoint, so plugging in a headset redirects the slider to the new device.
pub struct Volume {
    enumerator: IMMDeviceEnumerator,
}

impl Volume {
    /// Opens the device enumerator, or returns an error when COM is not ready.
    pub fn new() -> Result<Self> {
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&CLSID_MM_DEVICE_ENUMERATOR, None, CLSCTX_ALL)?;
            Ok(Self { enumerator })
        }
    }

    /// Opens the current default endpoint. Called on every operation so a
    /// device switch takes effect without recreating `Volume`.
    fn endpoint(&self) -> Result<IAudioEndpointVolume> {
        unsafe {
            let device = self.enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
            device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
        }
    }

    /// Normalized, audio-tapered level in `0.0..=1.0`.
    pub fn level(&self) -> Result<f32> {
        unsafe { self.endpoint()?.GetMasterVolumeLevelScalar() }
    }

    /// Sets the normalized level; values outside `0.0..=1.0` are clamped.
    pub fn set_level(&self, value: f32) -> Result<()> {
        unsafe { self.endpoint()?.SetMasterVolumeLevelScalar(value.clamp(0.0, 1.0), std::ptr::null()) }
    }

    /// Whether the endpoint is muted.
    pub fn muted(&self) -> Result<bool> {
        unsafe { Ok(self.endpoint()?.GetMute()?.as_bool()) }
    }

    /// Mutes or unmutes the endpoint.
    pub fn set_muted(&self, mute: bool) -> Result<()> {
        unsafe { self.endpoint()?.SetMute(mute, std::ptr::null()) }
    }
}

/// Display percent in `0..=100` from a normalized level.
pub fn percent(level: f32) -> u32 {
    (level.clamp(0.0, 1.0) * 100.0).round() as u32
}

/// Normalized level from a display percent in `0..=100`.
pub fn level(percent: u32) -> f32 {
    percent.min(100) as f32 / 100.0
}

/// Picks the AC or DC byte from the `[policy, ac, dc]` IOCTL response.
pub fn brightness_for_power(data: [u8; 3], on_ac: bool) -> u8 {
    if on_ac { data[1] } else { data[2] }
}

/// Opens `\\.\LCD` and reads the current panel brightness, or `None` when the
/// driver does not answer (desktops, external monitors, VMs).
pub fn brightness() -> Option<u8> {
    // shortcut: the \\.\LCD IOCTLs cover built-in laptop panels only; for
    // external monitors use WMI/DDC/CI when that becomes a requirement.
    unsafe {
        let handle = CreateFileW(&HSTRING::from(r"\\.\LCD"), 0,
            FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL, None).ok()?;
        let mut data = [0u8; 3];
        let mut bytes = 0u32;
        let read = DeviceIoControl(handle, IOCTL_VIDEO_QUERY_DISPLAY_BRIGHTNESS, None, 0,
            Some(data.as_mut_ptr().cast()), data.len() as u32, Some(&mut bytes), None).is_ok();
        _ = CloseHandle(handle);
        if !read || bytes < 3 {
            return None;
        }
        let mut status = SYSTEM_POWER_STATUS::default();
        let on_ac = GetSystemPowerStatus(&mut status).is_ok() && status.ACLineStatus == 1;
        Some(brightness_for_power(data, on_ac))
    }
}

/// Sets the panel brightness in `0..=100`. Returns whether the driver accepted.
pub fn set_brightness(value: u8) -> bool {
    unsafe {
        let handle = CreateFileW(&HSTRING::from(r"\\.\LCD"), 0,
            FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL, None);
        let Ok(handle) = handle else { return false };
        let value = value.min(100);
        let input = [3u8, value, value];
        let ok = DeviceIoControl(handle, IOCTL_VIDEO_SET_DISPLAY_BRIGHTNESS,
            Some(input.as_ptr().cast()), input.len() as u32, None, 0, None, None).is_ok();
        _ = CloseHandle(handle);
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_should_scale_and_clamp() {
        assert_eq!(percent(0.0), 0);
        assert_eq!(percent(0.5), 50);
        assert_eq!(percent(1.0), 100);
        assert_eq!(percent(1.5), 100);
        assert_eq!(percent(-1.0), 0);
    }

    #[test]
    fn level_should_scale_and_clamp() {
        assert_eq!(level(0), 0.0);
        assert_eq!(level(50), 0.5);
        assert_eq!(level(100), 1.0);
        assert_eq!(level(300), 1.0);
    }

    #[test]
    fn percent_and_level_should_round_trip() {
        for value in 0..=100 {
            assert_eq!(percent(level(value)), value);
        }
    }

    #[test]
    fn brightness_should_follow_the_power_source() {
        assert_eq!(brightness_for_power([0, 80, 40], true), 80);
        assert_eq!(brightness_for_power([0, 80, 40], false), 40);
    }

    #[test]
    fn clsid_should_match_mm_device_enumerator() {
        assert_eq!(
            CLSID_MM_DEVICE_ENUMERATOR,
            GUID::from_u128(0xbcde0395_e52f_467c_8e3d_c4579291692e)
        );
    }
}
