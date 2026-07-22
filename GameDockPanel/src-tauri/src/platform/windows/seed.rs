//! First-run seed candidate resolution for common gaming apps on Windows.

use std::path::{Path, PathBuf};

struct SeedResolver {
    name: &'static str,
    resolve: fn() -> Option<PathBuf>,
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).map(PathBuf::from)
}

fn path_exists(path: &Path) -> bool {
    path.is_file()
}

fn resolve_discord() -> Option<PathBuf> {
    let local = env_path("LOCALAPPDATA")?;
    let update_dir = local.join("Discord");
    let update_exe = update_dir.join("Update.exe");
    if path_exists(&update_exe) {
        return Some(update_exe);
    }
    let entries = std::fs::read_dir(&update_dir).ok()?;
    let mut candidates: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("app-"))
        })
        .collect();
    candidates.sort();
    for dir in candidates.into_iter().rev() {
        let exe = dir.join("Discord.exe");
        if path_exists(&exe) {
            return Some(exe);
        }
    }
    None
}

fn resolve_steam() -> Option<PathBuf> {
    if let Ok(path) = read_registry_string(
        windows::Win32::System::Registry::HKEY_CURRENT_USER,
        r"Software\Valve\Steam",
        "SteamPath",
    ) {
        let exe = PathBuf::from(path).join("steam.exe");
        if path_exists(&exe) {
            return Some(exe);
        }
    }
    for candidate in [
        r"C:\Program Files (x86)\Steam\steam.exe",
        r"C:\Program Files\Steam\steam.exe",
    ] {
        let p = PathBuf::from(candidate);
        if path_exists(&p) {
            return Some(p);
        }
    }
    None
}

fn resolve_spotify() -> Option<PathBuf> {
    let appdata = env_path("APPDATA")?;
    let exe = appdata.join("Spotify").join("Spotify.exe");
    path_exists(&exe).then_some(exe)
}

fn resolve_obs() -> Option<PathBuf> {
    for candidate in [
        r"C:\Program Files\obs-studio\bin\64bit\obs64.exe",
        r"C:\Program Files (x86)\obs-studio\bin\64bit\obs64.exe",
    ] {
        let p = PathBuf::from(candidate);
        if path_exists(&p) {
            return Some(p);
        }
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        let exe = PathBuf::from(pf)
            .join("obs-studio")
            .join("bin")
            .join("64bit")
            .join("obs64.exe");
        if path_exists(&exe) {
            return Some(exe);
        }
    }
    None
}

fn resolve_epic() -> Option<PathBuf> {
    for candidate in [
        r"C:\Program Files (x86)\Epic Games\Launcher\Portal\Binaries\Win64\EpicGamesLauncher.exe",
        r"C:\Program Files\Epic Games\Launcher\Portal\Binaries\Win64\EpicGamesLauncher.exe",
    ] {
        let p = PathBuf::from(candidate);
        if path_exists(&p) {
            return Some(p);
        }
    }
    None
}

fn resolve_battlenet() -> Option<PathBuf> {
    for candidate in [
        r"C:\Program Files (x86)\Battle.net\Battle.net Launcher.exe",
        r"C:\Program Files\Battle.net\Battle.net Launcher.exe",
    ] {
        let p = PathBuf::from(candidate);
        if path_exists(&p) {
            return Some(p);
        }
    }
    None
}

fn read_registry_string(
    root: windows::Win32::System::Registry::HKEY,
    subkey: &str,
    value_name: &str,
) -> Result<String, ()> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, KEY_READ, REG_VALUE_TYPE,
    };

    let subkey_wide: Vec<u16> = subkey.encode_utf16().chain([0]).collect();
    let value_wide: Vec<u16> = value_name.encode_utf16().chain([0]).collect();

    let mut hkey = HKEY::default();
    unsafe {
        RegOpenKeyExW(root, PCWSTR(subkey_wide.as_ptr()), Some(0), KEY_READ, &mut hkey)
            .ok()
            .map_err(|_| ())?;
    }

    let result = (|| {
        let mut data_type = REG_VALUE_TYPE::default();
        let mut buf_len: u32 = 0;
        unsafe {
            RegQueryValueExW(
                hkey,
                PCWSTR(value_wide.as_ptr()),
                None,
                Some(&mut data_type),
                None,
                Some(&mut buf_len),
            )
            .ok()
            .map_err(|_| ())?;
        }
        if buf_len < 2 {
            return Err(());
        }
        let mut buf: Vec<u16> = vec![0; (buf_len as usize) / 2];
        unsafe {
            RegQueryValueExW(
                hkey,
                PCWSTR(value_wide.as_ptr()),
                None,
                Some(&mut data_type),
                Some(buf.as_mut_ptr() as *mut u8),
                Some(&mut buf_len),
            )
            .ok()
            .map_err(|_| ())?;
        }
        let os = OsString::from_wide(&buf[..buf.len().saturating_sub(1)]);
        os.into_string().map_err(|_| ())
    })();

    unsafe {
        let _ = RegCloseKey(hkey);
    }
    result
}

const WINDOWS_SEED_RESOLVERS: &[SeedResolver] = &[
    SeedResolver {
        name: "Discord",
        resolve: resolve_discord,
    },
    SeedResolver {
        name: "Steam",
        resolve: resolve_steam,
    },
    SeedResolver {
        name: "Spotify",
        resolve: resolve_spotify,
    },
    SeedResolver {
        name: "OBS Studio",
        resolve: resolve_obs,
    },
    SeedResolver {
        name: "Epic Games",
        resolve: resolve_epic,
    },
    SeedResolver {
        name: "Battle.net",
        resolve: resolve_battlenet,
    },
];

/// Returns `(display_name, canonical_exe_path)` pairs for installed seed apps.
pub fn seed_app_candidates() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for resolver in WINDOWS_SEED_RESOLVERS {
        if let Some(path) = (resolver.resolve)() {
            if let Ok(canonical) = canonicalize_app_path(&path) {
                log::info!(
                    "seed: resolved {} -> {}",
                    resolver.name,
                    canonical
                );
                out.push((resolver.name.to_string(), canonical));
            }
        }
    }
    out
}

/// Strip `\\?\` / `\\?\UNC\` so paths match `QueryFullProcessImageNameW`.
pub fn strip_extended_path_prefix(path: &str) -> String {
    let lower = path.to_lowercase();
    if let Some(rest) = lower.strip_prefix(r"\\?\unc\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = lower.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        lower
    }
}

/// Canonical lowercase absolute path used as the stable app ID on Windows.
pub fn canonicalize_app_path(path: &Path) -> Result<String, String> {
    let canonical = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    Ok(strip_extended_path_prefix(&canonical.to_string_lossy()))
}

/// Normalize persisted dock app ids (strip `\\?\`) in place. Returns true if any changed.
pub fn normalize_persisted_app_ids(entries: &mut [crate::commands::apps::DockItem]) -> bool {
    use crate::commands::apps::DockItem;
    let mut changed = false;
    for item in entries.iter_mut() {
        let DockItem::App(entry) = item else {
            continue;
        };
        let new_id = strip_extended_path_prefix(&entry.bundle_id);
        if new_id != entry.bundle_id {
            entry.bundle_id = new_id.clone();
            entry.id = new_id;
            changed = true;
        } else if entry.id != entry.bundle_id {
            entry.id = entry.bundle_id.clone();
            changed = true;
        }
    }
    changed
}

/// Whether an app ID (canonical exe path) exists on disk.
pub fn is_app_installed(app_id: &str) -> bool {
    Path::new(app_id).is_file()
}
