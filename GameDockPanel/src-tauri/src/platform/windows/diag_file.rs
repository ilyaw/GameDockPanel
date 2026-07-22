//! Ultra-dense Windows diagnostic log (`tauri_windows_diagnostic.log`).
//!
//! Format: `[TIMESTAMP] [CONTEXT] [STATUS|ERR=0xXXXX] key=value …`
//! Complements the per-session `tauri-plugin-log` file; this sink is append-only
//! and focused on chrome / launch / WinAPI triage.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager};

static DIAG_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Resolve/create `app_log_dir()/tauri_windows_diagnostic.log` and remember it.
pub fn init(app: &AppHandle) -> Result<(), String> {
    let log_dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&log_dir).map_err(|e| e.to_string())?;
    let path = log_dir.join("tauri_windows_diagnostic.log");
    {
        let mut guard = DIAG_PATH
            .lock()
            .map_err(|e| format!("diag path lock: {e}"))?;
        *guard = Some(path.clone());
    }
    write_line("SETUP", "OK", &format!("path={}", path.display()));
    log::info!("[win-diag-file] {}", path.display());
    Ok(())
}

fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let millis = now.subsec_millis();
    // UTC wall clock via chrono-less formatting from epoch (good enough for triage).
    let days = secs / 86400;
    let day_secs = secs % 86400;
    let h = day_secs / 3600;
    let m = (day_secs % 3600) / 60;
    let s = day_secs % 60;
    // 1970-01-01 + days — approximate YYYY-MM-DD via civil_from_days
    let (y, mo, d) = civil_from_days(days as i64);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}.{millis:03}Z")
}

/// Algorithm from Howard Hinnant (public domain) — days since 1970-01-01 → Y-M-D.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

fn open_diag_file() -> Option<File> {
    let path = DIAG_PATH.lock().ok().and_then(|g| g.clone())?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()
}

/// Append one diagnostic line. Never panics; failures go to `log::warn`.
pub fn write_line(context: &str, status: &str, detail: &str) {
    let line = if detail.is_empty() {
        format!("[{}] [{}] [{}]", timestamp(), context, status)
    } else {
        format!("[{}] [{}] [{}] {}", timestamp(), context, status, detail)
    };
    // Mirror into the session log too for convenience.
    log::debug!("[win-diag-file] {line}");
    if let Some(mut file) = open_diag_file() {
        if let Err(err) = writeln!(file, "{line}") {
            log::warn!("[win-diag-file] write failed: {err}");
        }
    }
}

pub fn ok(context: &str, detail: impl AsRef<str>) {
    write_line(context, "OK", detail.as_ref());
}

pub fn err(context: &str, code: u32, detail: impl AsRef<str>) {
    write_line(context, &format!("ERR=0x{code:08X}"), detail.as_ref());
}

pub fn status(context: &str, status: &str, detail: impl AsRef<str>) {
    write_line(context, status, detail.as_ref());
}

/// Frontend `window.onerror` / unhandledrejection bridge.
pub fn log_frontend_error(message: String, source: Option<String>, line: Option<u32>) {
    write_line(
        "FRONTEND",
        "ERR",
        &format!(
            "message={message:?} source={:?} line={line:?}",
            source.unwrap_or_default()
        ),
    );
}
