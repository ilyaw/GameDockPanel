//! Small shared bits of the on-disk JSON persistence pattern used by both
//! `commands::apps` (`dock-apps.json`) and `commands::settings`
//! (`dock-settings.json`). Only the parts that are byte-for-byte identical
//! between the two live here — each domain keeps its own read/reseed logic
//! (e.g. `apps`'s corrupt-file log message), since unifying that too would
//! mean losing per-domain detail for no real gain.

use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

/// Resolves `app_data_dir()/file_name`, creating the directory if needed.
pub fn app_data_file(app: &AppHandle, file_name: &str) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(file_name))
}

/// Writes `value` to `path` atomically: a crash or force-quit mid-write
/// leaves the previous, still-valid file in place instead of a truncated
/// one — write to a sibling `.tmp` file, then `rename` (atomic on the same
/// filesystem) over the real path.
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(&tmp_path, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp_path, path).map_err(|e| e.to_string())?;
    Ok(())
}
