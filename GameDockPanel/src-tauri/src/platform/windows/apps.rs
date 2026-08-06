//! Windows app lifecycle — launch, quit, reveal, process monitoring.

use std::path::Path;

use tauri::{App, AppHandle, Emitter, Manager, WebviewWindow};
use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible, PostMessageW,
    SetForegroundWindow, SetWindowPos, ShowWindow, SWP_NOACTIVATE, SWP_NOZORDER, SW_RESTORE,
    WM_CLOSE,
};

use super::chrome::ChromeGuard;
use super::diag_file;
use super::icons::resolve_app_icon;
use super::launch;
use crate::commands::apps::{
    apply_icon_resolve, emit_apps_list_changed, AppsState, DockItem, SavedWindowFrame, ZoomState,
};
use crate::commands::settings::{DockPosition, SettingsState};
use crate::platform::geometry::{axis_css_dims, current_dock_position, current_icon_size_dip};

const RUNNING_RECONCILE_POLL_MS: u64 = 2000;
const LAUNCH_WATCH_DELAYS_MS: &[u64] = &[0, 150, 300, 600, 1200, 2400];

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
        Some("exe") => launch::normalize_launch_path(path),
        Some("lnk") => launch::resolve_lnk_target(path_obj),
        _ => Err(format!("unsupported path type: {path}")),
    }
}

pub fn activate_or_launch_app(app: AppHandle, app_id: String) -> Result<(), String> {
    let resolved = launch::normalize_launch_path(&app_id).unwrap_or_else(|_| app_id.clone());
    diag_file::status(
        "LAUNCH",
        "ACTIVATE_OR_LAUNCH",
        format!("raw={app_id} resolved={resolved}"),
    );

    if launch::is_explorer_path(&resolved) || launch::is_explorer_path(&app_id) {
        launch::launch_or_activate_explorer(&app)?;
        // Roster key drives AppsState.running / LED for this dock entry.
        start_launch_running_watch(app, app_id);
        return Ok(());
    }

    if live_app_running(&resolved) {
        if let Some(hwnd) = find_main_window_for_app(&resolved) {
            unsafe {
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                let _ = SetForegroundWindow(hwnd);
            }
            sync_bundle_running_state(&app, &resolved);
            if let Some(window) = app.get_webview_window("main") {
                ChromeGuard::on_surface_changed(&window);
            }
            diag_file::ok("LAUNCH", format!("activated existing hwnd={hwnd:?}"));
            return Ok(());
        }
    }

    // Also try the raw id in case normalize changed casing/path form.
    if resolved != app_id && live_app_running(&app_id) {
        if let Some(hwnd) = find_main_window_for_app(&app_id) {
            unsafe {
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                let _ = SetForegroundWindow(hwnd);
            }
            sync_bundle_running_state(&app, &app_id);
            if let Some(window) = app.get_webview_window("main") {
                ChromeGuard::on_surface_changed(&window);
            }
            return Ok(());
        }
    }

    launch::launch_exe(&app, &resolved)?;
    start_launch_running_watch(app, resolved);
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

pub fn reveal_app_in_finder(app: AppHandle, app_id: String) -> Result<(), String> {
    launch::reveal_in_explorer(&app, &app_id)
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

/// Zooms the target app's foreground window to fill the screen area above
/// (or beside) the dock pill; toggles back to the pre-zoom frame on repeat.
/// Mirrors `macos::zoom_app_above_dock` semantics (find window → compute
/// usable area minus the pill → save-and-fill, or restore if already
/// zoomed) using `GetWindowRect`/`SetWindowPos` instead of the Accessibility
/// API — Windows has no per-window AX equivalent, but every app here already
/// resolves to a single top-level HWND via `find_main_window_for_app`.
pub fn zoom_app_above_dock(app: AppHandle, app_id: String) -> Result<(), String> {
    diag_file::status("ZOOM", "BEGIN", format!("app_id={app_id}"));

    let Some(hwnd) = find_main_window_for_app(&app_id) else {
        let msg = format!("{app_id} is not running");
        diag_file::status("ZOOM", "ERR", &msg);
        return Err(msg);
    };

    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
    }

    let Some(dock_window) = app.get_webview_window("main") else {
        diag_file::status("ZOOM", "ERR", "main window missing");
        return Err("main window missing".to_string());
    };

    let usable = match usable_screen_rect_for_zoom(&dock_window) {
        Ok(rect) => rect,
        Err(err) => {
            diag_file::status("ZOOM", "ERR", format!("usable_screen_rect_for_zoom: {err}"));
            return Err(err);
        }
    };
    let current = match window_screen_rect(hwnd) {
        Ok(rect) => rect,
        Err(err) => {
            diag_file::status("ZOOM", "ERR", format!("window_screen_rect: {err}"));
            return Err(err);
        }
    };
    let is_zoomed = rects_approximately_equal(current, usable, ZOOM_FRAME_TOLERANCE_PX);
    diag_file::status(
        "ZOOM",
        "RECTS",
        format!("hwnd={hwnd:?} current={current:?} usable={usable:?} already_zoomed={is_zoomed}"),
    );

    let zoom_state = app.state::<ZoomState>();
    let storage_key = app_id;

    let result = if is_zoomed {
        let saved = {
            let mut guard = zoom_state
                .saved_frames
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.remove(&storage_key)
        };
        let Some(saved) = saved else {
            let msg = "window is already zoomed but no saved frame to restore".to_string();
            diag_file::status("ZOOM", "ERR", &msg);
            return Err(msg);
        };
        let target = ScreenRect {
            x: saved.x,
            y: saved.y,
            width: saved.width,
            height: saved.height,
        };
        diag_file::status("ZOOM", "RESTORE", format!("target={target:?}"));
        set_window_screen_rect(hwnd, target)
    } else {
        {
            let mut guard = zoom_state
                .saved_frames
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.insert(
                storage_key,
                SavedWindowFrame {
                    x: current.x,
                    y: current.y,
                    width: current.width,
                    height: current.height,
                },
            );
        }
        diag_file::status("ZOOM", "EXPAND", format!("target={usable:?}"));
        set_window_screen_rect(hwnd, usable)
    };

    match &result {
        Ok(()) => diag_file::ok("ZOOM", if is_zoomed { "restored" } else { "zoomed" }),
        Err(err) => diag_file::status("ZOOM", "ERR", format!("SetWindowPos: {err}")),
    }
    result
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
) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut FindWindowCtx);
    if ctx.found.is_some() {
        return BOOL(0);
    }
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    if ctx.pids.contains(&pid) && unsafe { IsWindowVisible(hwnd).as_bool() } {
        ctx.found = Some(hwnd);
        return BOOL(0);
    }
    BOOL(1)
}

/// Screen-space rect (physical px) — Windows equivalent of the `ScreenRect`
/// used by `macos::zoom_app_above_dock` for the same purpose.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ScreenRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl ScreenRect {
    fn right(&self) -> i32 {
        self.x + self.width
    }

    fn bottom(&self) -> i32 {
        self.y + self.height
    }
}

/// Gap between the zoomed window and the dock pill's near edge, mirroring
/// `ZOOM_DOCK_GAP_PX` in `macos.rs`.
const ZOOM_DOCK_GAP_PX: i32 = 4;
/// Frame comparison tolerance when deciding whether a window is already
/// zoomed, mirroring `ZOOM_FRAME_TOLERANCE_PX` in `macos.rs`.
const ZOOM_FRAME_TOLERANCE_PX: i32 = 4;

fn rects_approximately_equal(a: ScreenRect, b: ScreenRect, tolerance: i32) -> bool {
    (a.x - b.x).abs() <= tolerance
        && (a.y - b.y).abs() <= tolerance
        && (a.width - b.width).abs() <= tolerance
        && (a.height - b.height).abs() <= tolerance
}

fn window_screen_rect(hwnd: HWND) -> Result<ScreenRect, String> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }.map_err(|e| e.to_string())?;
    Ok(ScreenRect {
        x: rect.left,
        y: rect.top,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
    })
}

fn set_window_screen_rect(hwnd: HWND, rect: ScreenRect) -> Result<(), String> {
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    }
    .map_err(|e| format!("SetWindowPos(zoom) failed: {e}"))
}

/// Work area (screen minus taskbar) of the monitor the dock currently sits
/// on — the Win32 equivalent of `NSScreen.visibleFrame`.
fn monitor_work_rect(hwnd: HWND) -> Result<ScreenRect, String> {
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let ok = unsafe { GetMonitorInfoW(monitor, &mut info) };
    if !ok.as_bool() {
        return Err("failed to query monitor work area".to_string());
    }
    let work = info.rcWork;
    Ok(ScreenRect {
        x: work.left,
        y: work.top,
        width: work.right - work.left,
        height: work.bottom - work.top,
    })
}

/// Current dock pill rect on screen (physical px), always at rest size —
/// zoom targets the visual pill, not a transient hover-expanded HWND (the
/// dock is necessarily hovered when this runs, since zoom is triggered by
/// double-clicking an icon). Windows' resting HWND has no inside inset (see
/// `geometry::window_thickness_rest_dip`), so the pill's near edge always
/// sits flush with the corresponding HWND edge regardless of hover growth
/// on the far side — anchor from that edge instead of trusting the live
/// `outer_size` directly. Mirrors `macos::current_pill_screen_rect`.
fn current_pill_screen_rect(window: &WebviewWindow) -> Result<ScreenRect, String> {
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let outer_pos = window.outer_position().map_err(|e| e.to_string())?;
    let outer_size = window.outer_size().map_err(|e| e.to_string())?;
    let position = current_dock_position(window);

    let (pill_length_dip, pill_thickness_dip) = {
        let state = window.state::<AppsState>();
        let stored_width = *state
            .pill_width_dip
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let stored_height = *state
            .pill_height_dip
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        axis_css_dims(position.axis(), stored_width, stored_height)
    };
    let (pill_w_dip, pill_h_dip) =
        axis_css_dims(position.axis(), pill_length_dip, pill_thickness_dip);
    let pill_w = (pill_w_dip * scale).round() as i32;
    let pill_h = (pill_h_dip * scale).round() as i32;

    let (left, top) = match position {
        DockPosition::Bottom => (
            outer_pos.x + (outer_size.width as i32 - pill_w) / 2,
            outer_pos.y + outer_size.height as i32 - pill_h,
        ),
        DockPosition::Top => (
            outer_pos.x + (outer_size.width as i32 - pill_w) / 2,
            outer_pos.y,
        ),
        DockPosition::Left => (
            outer_pos.x,
            outer_pos.y + (outer_size.height as i32 - pill_h) / 2,
        ),
        DockPosition::Right => (
            outer_pos.x + outer_size.width as i32 - pill_w,
            outer_pos.y + (outer_size.height as i32 - pill_h) / 2,
        ),
    };
    Ok(ScreenRect {
        x: left,
        y: top,
        width: pill_w,
        height: pill_h,
    })
}

/// Shrinks the monitor's usable work area down to the free side of the
/// dock pill (plus `gap`). Mirrors `macos::shrink_visible_for_dock`.
fn shrink_visible_for_dock(
    visible: ScreenRect,
    pill: ScreenRect,
    position: DockPosition,
    gap: i32,
) -> Option<ScreenRect> {
    match position {
        DockPosition::Bottom => {
            let bottom = pill.y.saturating_sub(gap);
            let height = bottom.saturating_sub(visible.y);
            if height < 32 {
                return None;
            }
            Some(ScreenRect {
                x: visible.x,
                y: visible.y,
                width: visible.width,
                height,
            })
        }
        DockPosition::Top => {
            let top = pill.bottom().saturating_add(gap);
            let bottom = visible.bottom();
            let height = bottom.saturating_sub(top);
            if height < 32 {
                return None;
            }
            Some(ScreenRect {
                x: visible.x,
                y: top,
                width: visible.width,
                height,
            })
        }
        DockPosition::Left => {
            let left = pill.right().saturating_add(gap);
            let right = visible.right();
            let width = right.saturating_sub(left);
            if width < 32 {
                return None;
            }
            Some(ScreenRect {
                x: left,
                y: visible.y,
                width,
                height: visible.height,
            })
        }
        DockPosition::Right => {
            let right = pill.x.saturating_sub(gap);
            let width = right.saturating_sub(visible.x);
            if width < 32 {
                return None;
            }
            Some(ScreenRect {
                x: visible.x,
                y: visible.y,
                width,
                height: visible.height,
            })
        }
    }
}

fn usable_screen_rect_for_zoom(dock_window: &WebviewWindow) -> Result<ScreenRect, String> {
    let dock_hwnd = dock_window.hwnd().map_err(|e| e.to_string())?;
    let visible = monitor_work_rect(dock_hwnd)?;
    let pill = current_pill_screen_rect(dock_window)?;
    let position = current_dock_position(dock_window);
    diag_file::status(
        "ZOOM",
        "GEOMETRY",
        format!("position={position:?} monitor_work={visible:?} pill={pill:?}"),
    );
    shrink_visible_for_dock(visible, pill, position, ZOOM_DOCK_GAP_PX)
        .ok_or_else(|| "usable zoom area is too small".to_string())
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
) -> BOOL {
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
    BOOL(1)
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

/// Prefer Tauri window scale; fall back to HWND DPI / primary monitor.
/// Never default to `1.0` — that under-exports icons on HiDPI (macOS uses 2.0).
fn dock_window_scale_factor(app: &AppHandle) -> f64 {
    if let Some(window) = app.get_webview_window("main") {
        if let Ok(scale) = window.scale_factor() {
            if scale.is_finite() && scale > 0.0 {
                return scale;
            }
        }
        if let Ok(hwnd) = window.hwnd() {
            let dpi = unsafe { GetDpiForWindow(hwnd) };
            if dpi > 0 {
                return dpi as f64 / 96.0;
            }
        }
    }
    if let Ok(Some(monitor)) = app.primary_monitor() {
        let scale = monitor.scale_factor();
        if scale.is_finite() && scale > 0.0 {
            return scale;
        }
    }
    2.0
}
