//! Reversible shell registration under `HKEY_CURRENT_USER`.
//!
//! Only the per-user `Winlogon\Shell` value is changed; the machine-wide
//! `HKLM\...\Winlogon\Shell` (normally `explorer.exe`) is never touched. The
//! previous value is saved under `HKCU\Software\RWindows` so uninstall can put
//! it back. This is a community-documented mechanism, not an official API:
//! verify on a VM before using it on a daily machine.

use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ, RegCloseKey, RegCreateKeyExW,
    RegDeleteValueW, RegEnumValueW, RegOpenKeyExW, RegQueryInfoKeyW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{HSTRING, Result};

/// Real per-user Winlogon key.
pub const WINLOGON: &str = r"Software\Microsoft\Windows NT\CurrentVersion\Winlogon";
/// Real backup key owned by this project.
pub const SETTINGS: &str = r"Software\RWindows";

const SHELL: &str = "Shell";
const ORIGINAL: &str = "OriginalShell";

/// Paths used by [`ShellRegistration`]. Tests point these at throwaway keys.
pub struct ShellRegistration {
    /// Key holding the `Shell` value.
    pub winlogon: HSTRING,
    /// Key holding the saved original value.
    pub settings: HSTRING,
}

impl Default for ShellRegistration {
    fn default() -> Self {
        Self { winlogon: HSTRING::from(WINLOGON), settings: HSTRING::from(SETTINGS) }
    }
}

fn open(root: HKEY, subkey: &HSTRING, write: bool) -> Result<HKEY> {
    unsafe {
        let mut key = HKEY::default();
        if write {
            RegCreateKeyExW(root, subkey, None, None, Default::default(), KEY_WRITE,
                None, &mut key, None).ok()?;
        } else {
            RegOpenKeyExW(root, subkey, None, KEY_READ, &mut key).ok()?;
        }
        Ok(key)
    }
}

/// Reads a REG_SZ value, or `None` when the value or key is absent.
pub fn read(root: HKEY, subkey: &HSTRING, value: &str) -> Result<Option<String>> {
    let key = match open(root, subkey, false) {
        Ok(key) => key,
        Err(_) => return Ok(None),
    };
    unsafe {
        let name = HSTRING::from(value);
        let mut kind = Default::default();
        let mut size = 0u32;
        let status = RegQueryValueExW(key, &name, None, Some(&mut kind), None, Some(&mut size));
        if status != WIN32_ERROR(0) || kind != REG_SZ || size == 0 {
            _ = RegCloseKey(key);
            return Ok(None);
        }
        let mut bytes = vec![0u8; size as usize];
        let status = RegQueryValueExW(key, &name, None, None, Some(bytes.as_mut_ptr()), Some(&mut size));
        _ = RegCloseKey(key);
        if status != WIN32_ERROR(0) {
            return Ok(None);
        }
        let units = size as usize / 2;
        let text: Vec<u16> = bytes.as_chunks::<2>().0.iter()
            .take(units)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        Ok(String::from_utf16_lossy(&text).trim_end_matches('\0').to_owned().into())
    }
}

/// Names of all values in a key, used to enumerate the `Run` key.
pub fn value_names(root: HKEY, subkey: &HSTRING) -> Result<Vec<String>> {
    let key = open(root, subkey, false)?;
    unsafe {
        let mut count = 0u32;
        RegQueryInfoKeyW(key, None, None, None, None, None, None,
            Some(&mut count), None, None, None, None).ok()?;
        let mut names = Vec::with_capacity(count as usize);
        for index in 0..count {
            let mut buffer = [0u16; 256];
            let mut length = buffer.len() as u32;
            RegEnumValueW(key, index, Some(windows::core::PWSTR(buffer.as_mut_ptr())), &mut length,
                None, None, None, None).ok()?;
            names.push(String::from_utf16_lossy(&buffer[..length as usize]));
        }
        _ = RegCloseKey(key);
        Ok(names)
    }
}

/// Writes a REG_SZ value, creating the key if needed.
pub fn write(root: HKEY, subkey: &HSTRING, value: &str, data: &str) -> Result<()> {
    unsafe {
        let mut key = HKEY::default();
        RegCreateKeyExW(root, subkey, None, None, Default::default(), KEY_WRITE,
            None, &mut key, None).ok()?;
        let mut units: Vec<u16> = data.encode_utf16().collect();
        // REG_SZ must include the terminating null in cbData (MSDN).
        units.push(0);
        let bytes: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
        let result = RegSetValueExW(key, &HSTRING::from(value), None, REG_SZ, Some(&bytes)).ok();
        _ = RegCloseKey(key);
        result
    }
}

fn remove(root: HKEY, subkey: &HSTRING, value: &str) -> Result<()> {
    let key = open(root, subkey, true)?;
    unsafe {
        let status = RegDeleteValueW(key, &HSTRING::from(value));
        _ = RegCloseKey(key);
        // Removing an already-absent value is success for our callers.
        if status == WIN32_ERROR(2) { Ok(()) } else { status.ok() }
    }
}

impl ShellRegistration {
    /// Current `Shell` value, if one is set.
    pub fn current(&self) -> Result<Option<String>> {
        read(HKEY_CURRENT_USER, &self.winlogon, SHELL)
    }

    /// Saves the current shell and points Winlogon at `exe`. Idempotent.
    pub fn install(&self, exe: &str) -> Result<()> {
        let current = self.current()?;
        if current.as_deref() == Some(exe) {
            return Ok(());
        }
        match current {
            Some(value) => write(HKEY_CURRENT_USER, &self.settings, ORIGINAL, &value)?,
            None => _ = remove(HKEY_CURRENT_USER, &self.settings, ORIGINAL),
        }
        write(HKEY_CURRENT_USER, &self.winlogon, SHELL, exe)
    }

    /// Restores the saved shell; when none was saved, removes our value so
    /// Windows falls back to `explorer.exe`.
    pub fn uninstall(&self) -> Result<()> {
        match read(HKEY_CURRENT_USER, &self.settings, ORIGINAL)? {
            Some(original) => write(HKEY_CURRENT_USER, &self.winlogon, SHELL, &original)?,
            None => {
                if open(HKEY_CURRENT_USER, &self.winlogon, true).is_ok() {
                    remove(HKEY_CURRENT_USER, &self.winlogon, SHELL)?;
                }
            }
        }
        if open(HKEY_CURRENT_USER, &self.settings, true).is_ok() {
            _ = remove(HKEY_CURRENT_USER, &self.settings, ORIGINAL);
        }
        Ok(())
    }
}

/// Quotes a path with spaces for the Winlogon `Shell` command line.
pub fn command_line(exe: &str) -> String {
    if exe.contains(' ') && !(exe.starts_with('"') && exe.ends_with('"')) {
        format!("\"{exe}\"")
    } else {
        exe.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_WINLOGON: &str = r"Software\RWindowsTest\Winlogon";
    const TEST_SETTINGS: &str = r"Software\RWindowsTest\Settings";

    fn fixture(name: &str) -> ShellRegistration {
        ShellRegistration {
            winlogon: HSTRING::from(format!("{TEST_WINLOGON}\\{name}")),
            settings: HSTRING::from(format!("{TEST_SETTINGS}\\{name}")),
        }
    }

    #[test]
    fn install_saves_the_old_shell_and_uninstall_restores_it() {
        let reg = fixture("restore");
        reg.uninstall().unwrap(); // start clean
        write(HKEY_CURRENT_USER, &reg.winlogon, SHELL, "explorer.exe").unwrap();
        reg.install("C:\\RWindows\\shell.exe").unwrap();
        assert_eq!(reg.current().unwrap().as_deref(), Some("C:\\RWindows\\shell.exe"));
        assert_eq!(read(HKEY_CURRENT_USER, &reg.settings, ORIGINAL).unwrap().as_deref(),
            Some("explorer.exe"));
        reg.uninstall().unwrap();
        assert_eq!(reg.current().unwrap().as_deref(), Some("explorer.exe"));
        reg.uninstall().unwrap();
    }

    #[test]
    fn install_is_idempotent_and_keeps_the_original() {
        let reg = fixture("idempotent");
        write(HKEY_CURRENT_USER, &reg.winlogon, SHELL, "explorer.exe").unwrap();
        reg.install("shell.exe").unwrap();
        reg.install("shell.exe").unwrap();
        assert_eq!(read(HKEY_CURRENT_USER, &reg.settings, ORIGINAL).unwrap().as_deref(),
            Some("explorer.exe"));
        reg.uninstall().unwrap();
    }

    #[test]
    fn command_line_quotes_paths_with_spaces() {
        assert_eq!(command_line("C:\\Program Files\\shell.exe"), "\"C:\\Program Files\\shell.exe\"");
        assert_eq!(command_line("shell.exe"), "shell.exe");
    }
}
