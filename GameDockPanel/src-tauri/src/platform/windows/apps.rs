//! Windows app lifecycle — launch, quit, reveal, process monitoring.

use std::path::Path;

use tauri::{App, AppHandle, Emitter, Manager};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::OpenProcess;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsIconic, IsWindowVisible, PostMessageW, ShowWindow,
    SW_RESTORE, WM_CLOSE,
};

use super::icons::resolve_app_icon;
use super::seed::{canonicalize_app_path, is_app_installed};
use crate::commands::apps::{apply_icon_resolve, emit_apps_list_changed, AppsState, DockItem};
use crate::commands::settings::SettingsState;
use crate::platform::geometry::current_icon_size_dip;

const RUNNING_RECONCILE_POLL_MS: u64 = 2000;
const LAUNCH_WATCH_DELAYS_MS: &[u64] = &[0, 150, 300, 600, 1200, 2400];

pub use super::seed::seed_app_candidates;

pub fn is_bundle_running(app_id: &str) -> bool {
    live_app_running(app_id)
}

pub fn resolve_bundle_id_from_path(path: &str) -> Result<String, String> {
    let path_obj = Path::new(path);
    let ext = path_obj
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());

    match ext.as_deref() {
        Some("exe") => canonicalize_app_path(path_obj),
        Some("lnk") => resolve_lnk_target(path_obj),
        _ => Err(format!("unsupported path type: {path}")),
    }
}

fn resolve_lnk_target(lnk: &Path) -> Result<String, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::Interface;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{IPersistFile, IShellLinkW, ShellLink};

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }

    let shell_link: IShellLinkW = unsafe {
        CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?
    };
    let persist: IPersistFile = shell_link.cast().map_err(|e| e.to_string())?;

    let wide: Vec<u16> = lnk.as_os_str().encode_wide().chain([0]).collect();
    unsafe {
        persist
            .Load(PCWSTR(wide.as_ptr()), windows::Win32::System::Com::STGM_READ)
            .map_err(|e| e.to_string())?;
    }

    let mut target_buf = [0u16; 1024];
    unsafe {
        shell_link
            .GetPath(&mut target_buf, std::ptr::null_mut(), 0)
            .map_err(|e| e.to_string())?;
    }

    let len = target_buf.iter().position(|&c| c == 0).unwrap_or(0);
    let target = String::from_utf16_lossy(&target_buf[..len]);
    canonicalize_app_path(Path::new(&target))
}

pub fn activate_or_launch_app(app: AppHandle, app_id: String) -> Result<(), String> {
    if live_app_running(&app_id) {
        if let Some(hwnd) = find_main_window_for_app(&app_id) {
            unsafe {
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(hwnd);
            }
            sync_bundle_running_state(&app, &app_id);
            return Ok(());
        }
    }

    let path_wide: Vec<u16> = app_id.encode_utf16().chain([0]).collect();
    let dir = Path::new(&app_id)
        .parent()
        .map(|p| p.to_string_lossy().encode_utf16().chain([0]).collect::<Vec<_>>())
        .unwrap_or_else(|| vec![0]);

    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR::null(),
            PCWSTR(path_wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR(dir.as_ptr()),
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        )
    };

    if result.0 as isize <= 32 {
        return Err(format!("ShellExecuteW failed for {app_id}"));
    }

    start_launch_running_watch(app, app_id);
    Ok(())
}

pub fn quit_app(_app: AppHandle, app_id: String) -> Result<(), String> {
    let pids = pids_for_app(&app_id);
    if pids.is_empty() {
        return Err(format!("no running process for {app_id}"));
    }
    for pid in pids {
        post_close_to_process_windows(pid);
    }
    Ok(())
}

pub fn reveal_app_in_finder(_app: AppHandle, app_id: String) -> Result<(), String> {
    let path = Path::new(&app_id);
    if !path.is_file() {
        return Err(format!("path not found: {app_id}"));
    }
    let arg = format!("/select,\"{}\"", path.display());
    let arg_wide: Vec<u16> = arg.encode_utf16().chain([0]).collect();
    let explorer: Vec<u16> = "explorer".encode_utf16().chain([0]).collect();
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR::null(),
            PCWSTR(explorer.as_ptr()),
            PCWSTR(arg_wide.as_ptr()),
            PCWSTR::null(),
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        return Err("failed to open Explorer".to_string());
    }
    Ok(())
}

pub fn refresh_dock_icons(app: &AppHandle, state: &AppsState) {
    let icon_size_dip = current_icon_size_dip_from_app(app);
    let scale_factor = dock_window_scale_factor(app);
    let entries = state.entries_snapshot();

    for item in entries.iter() {
        let DockItem::App(entry) = item else {
            continue;
        };
        let resolved = resolve_app_icon(&entry.bundle_id, icon_size_dip, scale_factor);
        apply_icon_resolve(state, &entry.bundle_id, resolved);
    }

    emit_apps_icons_updated(app, state.icons_snapshot());
    emit_apps_list_changed(app, state);
}

pub fn start_apps_monitoring(app: &App) -> Result<(), String> {
    let app_handle = app.handle().clone();
    let state = app.state::<AppsState>();

    {
        let entries = state
            .entries
            .lock()
            .map_err(|e| e.to_string())?;
        let mut running = state
            .running
            .lock()
            .map_err(|e| e.to_string())?;
        for item in entries.iter() {
            let DockItem::App(entry) = item else {
                continue;
            };
            running.insert(entry.bundle_id.clone(), live_app_running(&entry.bundle_id));
        }
        log::info!("apps monitoring: seeded running state for {} apps", running.len());
    }

    {
        let entries = state
            .entries
            .lock()
            .map_err(|e| e.to_string())?;
        let icon_size_dip = current_icon_size_dip_from_app(&app_handle);
        let scale_factor = dock_window_scale_factor(&app_handle);
        for item in entries.iter() {
            let DockItem::App(entry) = item else {
                continue;
            };
            let resolved = resolve_app_icon(&entry.bundle_id, icon_size_dip, scale_factor);
            apply_icon_resolve(&state, &entry.bundle_id, resolved);
        }
    }

    start_running_reconcile_poller(app_handle.clone());

    emit_apps_icons_updated(&app_handle, app.state::<AppsState>().icons_snapshot());
    emit_apps_running_changed(&app_handle);
    emit_apps_list_changed(&app_handle, &app.state::<AppsState>());

    Ok(())
}

pub fn zoom_app_above_dock(_app: AppHandle, _app_id: String) -> Result<(), String> {
    Err("dock zoom is not implemented on Windows yet".to_string())
}

fn live_app_running(app_id: &str) -> bool {
    !pids_for_app(app_id).is_empty()
}

fn pids_for_app(app_id: &str) -> Vec<u32> {
    let target = app_id.to_lowercase();
    let mut pids = Vec::new();
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return pids;
    };
    if snapshot.is_invalid() {
        return pids;
    }

    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };

    let mut ok = unsafe { Process32FirstW(snapshot, &mut entry).is_ok() };
    while ok {
        if let Some(image_path) = process_image_path(entry.th32ProcessID) {
            if image_path.to_lowercase() == target {
                pids.push(entry.th32ProcessID);
            }
        }
        ok = unsafe { Process32NextW(snapshot, &mut entry).is_ok() };
    }

    unsafe {
        let _ = CloseHandle(snapshot);
    }
    pids
}

fn process_image_path(pid: u32) -> Option<String> {
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let handle = unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?
    };
    let result = (|| {
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buf.as_mut_ptr()),
                &mut size,
            )
            .ok()?;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
        Some(String::from_utf16_lossy(&buf[..len]))
    })();
    unsafe {
        let _ = CloseHandle(handle);
    }
    result
}

struct FindWindowCtx {
    pids: Vec<u32>,
    found: Option<HWND>,
}

struct CloseWindowsCtx {
    pid: u32,
}

fn find_main_window_for_app(app_id: &str) -> Option<HWND> {
    let pids = pids_for_app(app_id);
    if pids.is_empty() {
        return None;
    }
    let mut ctx = FindWindowCtx { pids, found: None };
    unsafe {
        let _ = EnumWindows(
            Some(enum_window_callback),
            LPARAM(&mut ctx as *mut FindWindowCtx as isize),
        );
    }
    ctx.found
}

unsafe extern "system" fn enum_window_callback(
    hwnd: HWND,
    lparam: LPARAM,
) -> windows::Win32::Foundation::BOOL {
    let ctx = &mut *(lparam.0 as *mut FindWindowCtx);
    if ctx.found.is_some() {
        return windows::Win32::Foundation::BOOL(0);
    }
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if ctx.pids.contains(&pid) && unsafe { IsWindowVisible(hwnd).as_bool() } {
        ctx.found = Some(hwnd);
        return windows::Win32::Foundation::BOOL(0);
    }
    windows::Win32::Foundation::BOOL(1)
}

fn post_close_to_process_windows(pid: u32) {
    let ctx = CloseWindowsCtx { pid };
    unsafe {
        let _ = EnumWindows(
            Some(close_windows_callback),
            LPARAM(&ctx as *const CloseWindowsCtx as isize),
        );
    }
}

unsafe extern "system" fn close_windows_callback(
    hwnd: HWND,
    lparam: LPARAM,
) -> windows::Win32::Foundation::BOOL {
    let ctx = &*(lparam.0 as *const CloseWindowsCtx);
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if pid == ctx.pid {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
    windows::Win32::Foundation::BOOL(1)
}

fn sync_bundle_running_state(app: &AppHandle, app_id: &str) -> bool {
    let live = live_app_running(app_id);
    let state = app.state::<AppsState>();
    let changed = {
        let mut running = state
            .running
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let stored = running.get(app_id).copied().unwrap_or(false);
        if stored != live {
            running.insert(app_id.to_string(), live);
            true
        } else {
            false
        }
    };
    if changed {
        emit_apps_running_changed(app);
    }
    live
}

fn start_launch_running_watch(app: AppHandle, app_id: String) {
    std::thread::spawn(move || {
        for &delay_ms in LAUNCH_WATCH_DELAYS_MS {
            if delay_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            }
            if sync_bundle_running_state(&app, &app_id) {
                break;
            }
        }
    });
}

fn start_running_reconcile_poller(app_handle: AppHandle) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_millis(
                RUNNING_RECONCILE_POLL_MS,
            ));

            let candidates: Vec<String> = {
                let state = app_handle.state::<AppsState>();
                let entries = state
                    .entries
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                entries
                    .iter()
                    .filter_map(|item| {
                        let DockItem::App(entry) = item else {
                            return None;
                        };
                        Some(entry.bundle_id.clone())
                    })
                    .collect()
            };

            if candidates.is_empty() {
                continue;
            }

            let mut changed = false;
            let state = app_handle.state::<AppsState>();
            for app_id in candidates {
                let live = live_app_running(&app_id);
                let mut running = state
                    .running
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let stored = running.get(&app_id).copied().unwrap_or(false);
                if stored != live {
                    running.insert(app_id, live);
                    changed = true;
                }
            }
            if changed {
                emit_apps_running_changed(&app_handle);
            }
        }
    });
}

fn emit_apps_running_changed(app: &AppHandle) {
    let state = app.state::<AppsState>();
    let payload = state.running_snapshot();
    let _ = app.emit("apps-running-changed", payload);
}

fn emit_apps_icons_updated(
    app: &AppHandle,
    updates: Vec<crate::commands::apps::AppIconUpdatePayload>,
) {
    let _ = app.emit("apps-icons-updated", updates);
}

fn current_icon_size_dip_from_app(app: &AppHandle) -> f64 {
    if let Some(window) = app.get_webview_window("main") {
        return current_icon_size_dip(&window);
    }
    let state = app.state::<SettingsState>();
    let guard = state
        .settings
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.icon_size_px
}

fn dock_window_scale_factor(app: &AppHandle) -> f64 {
    app.get_webview_window("main")
        .and_then(|window| window.scale_factor().ok())
        .unwrap_or(1.0)
}
