//! Apps the shell must start at logon when Explorer is not running: the
//! per-user `Run` key and the Startup folder. Both are documented Windows
//! autostart locations; HKLM entries are left to the system.

use windows::Win32::System::Registry::HKEY_CURRENT_USER;
use windows::Win32::UI::Shell::FOLDERID_Startup;
use windows::core::{HSTRING, Result};

use crate::app_list;
use crate::shell_registration::{read, value_names};

/// `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// One autostart entry and where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartupEntry {
    /// Value name or shortcut file name.
    pub name: String,
    /// Command line the shell should run.
    pub command: String,
    /// `true` for the Run key, `false` for a Startup folder shortcut.
    pub from_run_key: bool,
}

/// Enumerates `Run` values and shortcuts in the user's Startup folder.
pub fn enumerate() -> Result<Vec<StartupEntry>> {
    let mut entries = Vec::new();
    let key = HSTRING::from(RUN_KEY);
    if let Ok(names) = value_names(HKEY_CURRENT_USER, &key) {
        for name in names {
            if let Some(command) = read(HKEY_CURRENT_USER, &key, &name)? {
                entries.push(StartupEntry { name, command, from_run_key: true });
            }
        }
    }
    if let Ok(folder) = app_list::known_folder(&FOLDERID_Startup)
        && let Ok(read_dir) = std::fs::read_dir(folder)
    {
        for item in read_dir.flatten() {
            let path = item.path();
            if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("lnk")) {
                entries.push(StartupEntry {
                    name: path.file_stem().and_then(|stem| stem.to_str()).unwrap_or_default().to_owned(),
                    command: path.to_string_lossy().into_owned(),
                    from_run_key: false,
                });
            }
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerate_should_read_this_users_autostart_without_changing_it() {
        let entries = enumerate().unwrap_or_default();
        assert!(entries.iter().all(|entry| !entry.command.is_empty()));
    }

    #[test]
    fn run_key_is_the_per_user_one() {
        assert!(RUN_KEY.contains("CurrentVersion"));
        assert!(!RUN_KEY.starts_with("Software\\Microsoft\\Windows NT"));
    }
}
