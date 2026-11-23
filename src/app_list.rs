//! Apps for the Start menu: shortcuts from the user and common Start Menu
//! folders, with the localized display name Windows gives each shortcut.
//! Pin state lives in an undocumented Start layout database, so this is the
//! installed-app list, not an exact copy of the user's pinned grid.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::UI::Shell::{
    FOLDERID_CommonStartMenu, FOLDERID_Recent, FOLDERID_StartMenu, SHFILEINFOW, SHGFI_DISPLAYNAME,
    SHGetFileInfoW, SHGetKnownFolderPath,
};
use windows::core::{HSTRING, Result};

/// One app entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppInfo {
    /// Localized name shown in Start.
    pub name: String,
    /// Shortcut or executable path.
    pub path: PathBuf,
}

/// True for files Windows exposes as apps in Start.
pub fn is_app_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("lnk") || e.eq_ignore_ascii_case("exe")),
        Some(true)
    )
}

/// Fallback name from a file name without its extension.
pub fn name_from_path(path: &Path) -> String {
    path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_owned()
}

fn known_folder(id: *const windows::core::GUID) -> Result<PathBuf> {
    unsafe {
        let path = SHGetKnownFolderPath(id, Default::default(), None)?;
        let text = path.to_string()?;
        CoTaskMemFree(Some(path.0.cast()));
        Ok(PathBuf::from(text))
    }
}

/// Start Menu folders, user first (user entries override common ones).
pub fn folders() -> Result<Vec<PathBuf>> {
    let mut folders = Vec::new();
    for id in [&FOLDERID_StartMenu as *const _, &FOLDERID_CommonStartMenu as *const _] {
        let root = known_folder(id)?;
        for sub in ["", "Programs"] {
            let folder = if sub.is_empty() { root.clone() } else { root.join(sub) };
            if folder.is_dir() && !folders.contains(&folder) {
                folders.push(folder);
            }
        }
    }
    Ok(folders)
}

/// Name Windows shows for `path`, or the file stem when Shell has no entry.
pub fn display_name(path: &Path) -> String {
    let text = path.to_string_lossy();
    let mut info = SHFILEINFOW::default();
    unsafe {
        let ok = SHGetFileInfoW(&HSTRING::from(text.as_ref()), FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info), size_of::<SHFILEINFOW>() as u32, SHGFI_DISPLAYNAME);
        if ok != 0 {
            let name = &info.szDisplayName;
            let len = name.iter().position(|&unit| unit == 0).unwrap_or(name.len());
            if len > 0 {
                return String::from_utf16_lossy(&name[..len]);
            }
        }
    }
    name_from_path(path)
}

/// Installed apps from the Start Menu folders, deduplicated by path and sorted
/// by name. `Programs` subfolders are included one level deep.
pub fn enumerate() -> Result<Vec<AppInfo>> {
    let mut apps: Vec<AppInfo> = Vec::new();
    for folder in folders()? {
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let candidates: Vec<PathBuf> = if path.is_dir() {
                match std::fs::read_dir(&path) {
                    Ok(children) => children.flatten().map(|child| child.path()).collect(),
                    Err(_) => Vec::new(),
                }
            } else {
                vec![path]
            };
            for path in candidates {
                if !is_app_file(&path) || apps.iter().any(|app| app.path == path) {
                    continue;
                }
                apps.push(AppInfo { name: display_name(&path), path });
            }
        }
    }
    apps.sort_by_key(|app| app.name.to_lowercase());
    Ok(apps)
}

/// Six recent items as in the Windows 11 Start menu. Reads `FOLDERID_Recent`
/// (shortcuts to recently opened documents), newest first.
pub fn recent() -> Result<Vec<AppInfo>> {
    let root = known_folder(&FOLDERID_Recent)?;
    let mut items = Vec::new();
    for entry in std::fs::read_dir(root)?.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("lnk")) {
            let modified = entry.metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            items.push((AppInfo { name: display_name(&path), path }, modified));
        }
    }
    Ok(newest(items, 6))
}

/// Keeps the newest `limit` items, newest first.
pub fn newest<T>(mut items: Vec<(T, SystemTime)>, limit: usize) -> Vec<T> {
    items.sort_by_key(|item| std::cmp::Reverse(item.1));
    items.into_iter().take(limit).map(|(item, _)| item).collect()
}

/// Placeholder text for the Start search box.
pub const SEARCH_PLACEHOLDER: &str = "Pesquisar aplicativos, arquivos e configurações";

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn app_file_filter_should_accept_shortcuts_and_executables() {
        assert!(is_app_file(Path::new("Chrome.lnk")));
        assert!(is_app_file(Path::new("APP.EXE")));
        assert!(!is_app_file(Path::new("readme.txt")));
        assert!(!is_app_file(Path::new("folder")));
    }

    #[test]
    fn fallback_name_should_strip_the_extension() {
        assert_eq!(name_from_path(Path::new(r"C:\Apps\Editor.lnk")), "Editor");
    }

    #[test]
    fn newest_should_sort_by_time_and_cap_the_count() {
        let base = SystemTime::UNIX_EPOCH;
        let items = vec![
            ("old", base),
            ("newest", base + Duration::from_secs(30)),
            ("middle", base + Duration::from_secs(10)),
        ];
        assert_eq!(newest(items, 2), vec!["newest", "middle"]);
    }

    #[test]
    fn search_placeholder_should_match_the_win11_label() {
        assert_eq!(SEARCH_PLACEHOLDER, "Pesquisar aplicativos, arquivos e configurações");
    }
}
