//! Canonical Windows process launch — ShellExecuteEx, env expansion, Explorer.

use std::mem::zeroed;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use tauri::{AppHandle, Manager};
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, HWND, LPARAM, BOOL};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    STGM_READ,
};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::System::Threading::GetProcessId;
use windows::Win32::UI::Shell::{
    IShellLinkW, ShellExecuteExW, ShellLink, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOCLOSEPROCESS,
    SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetForegroundWindow, IsIconic, IsWindowVisible, SetForegroundWindow,
    ShowWindow, SW_RESTORE, SW_SHOWNORMAL,
};

use super::chrome::ChromeGuard;
use super::diag_file;
use super::seed::canonicalize_app_path;

/// Expand `%VAR%`, trim, and canonicalize when the file exists.
pub fn normalize_launch_path(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("empty launch path".into());
    }
    let expanded = expand_env_strings(trimmed)?;
    let path = PathBuf::from(&expanded);
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
    {
        return resolve_lnk_target(&path);
    }
    if path.is_file() {
        return canonicalize_app_path(&path);
    }
    Ok(expanded.to_lowercase())
}

pub fn is_explorer_path(path: &str) -> bool {
    let lower = path.to_lowercase().replace('/', "\\");
    lower.ends_with("\\explorer.exe") || lower == "explorer.exe" || lower == "explorer"
}

fn expand_env_strings(input: &str) -> Result<String, String> {
    let wide: Vec<u16> = Path::new(input)
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect();
    let needed = unsafe { ExpandEnvironmentStringsW(PCWSTR(wide.as_ptr()), None) };
    if needed == 0 {
        let gle = unsafe { GetLastError().0 };
        diag_file::err("WINAPI", gle, "ExpandEnvironmentStringsW size query failed");
        return Err(format!("ExpandEnvironmentStringsW failed gle={gle}"));
    }
    let mut buf = vec![0u16; needed as usize];
    let written = unsafe { ExpandEnvironmentStringsW(PCWSTR(wide.as_ptr()), Some(&mut buf)) };
    if written == 0 {
        let gle = unsafe { GetLastError().0 };
        diag_file::err("WINAPI", gle, "ExpandEnvironmentStringsW expand failed");
        return Err(format!("ExpandEnvironmentStringsW failed gle={gle}"));
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Ok(String::from_utf16_lossy(&buf[..len]))
}

pub fn resolve_lnk_target(lnk: &Path) -> Result<String, String> {
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
            .Load(PCWSTR(wide.as_ptr()), STGM_READ)
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
    if target.is_empty() {
        return Err(format!(".lnk has empty target: {}", lnk.display()));
    }
    let expanded = expand_env_strings(&target)?;
    canonicalize_app_path(Path::new(&expanded))
}

fn explorer_exe_path() -> String {
    expand_env_strings("%WINDIR%\\explorer.exe")
        .unwrap_or_else(|_| r"C:\Windows\explorer.exe".into())
}

struct ExplorerFindState {
    found: HWND,
}

unsafe extern "system" fn enum_explorer_windows(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let state = &mut *(lparam.0 as *mut ExplorerFindState);
    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }
    let mut buf = [0u16; 64];
    let len = GetClassNameW(hwnd, &mut buf);
    if len <= 0 {
        return BOOL(1);
    }
    let class = String::from_utf16_lossy(&buf[..len as usize]);
    if class == "CabinetWClass" || class == "ExploreWClass" {
        state.found = hwnd;
        return BOOL(0);
    }
    BOOL(1)
}

fn find_explorer_window() -> Option<HWND> {
    let mut state = ExplorerFindState {
        found: HWND::default(),
    };
    let _ = unsafe {
        EnumWindows(
            Some(enum_explorer_windows),
            LPARAM(&mut state as *mut ExplorerFindState as isize),
        )
    };
    if state.found.is_invalid() {
        None
    } else {
        Some(state.found)
    }
}

/// Activate an existing Explorer window, or open a new one cleanly.
pub fn launch_or_activate_explorer(app: &AppHandle) -> Result<(), String> {
    diag_file::status("EXPLORER", "BEGIN", "activate_or_open");
    if let Some(hwnd) = find_explorer_window() {
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
        diag_file::ok("EXPLORER", format!("activated hwnd={hwnd:?}"));
        reassert_dock_after_launch(app);
        return Ok(());
    }

    let exe = explorer_exe_path();
    diag_file::status("EXPLORER", "COLD_START", format!("exe={exe}"));
    match Command::new(&exe).spawn() {
        Ok(child) => {
            diag_file::ok(
                "EXPLORER",
                format!("spawn pid={}", child.id()),
            );
            reassert_dock_after_launch(app);
            Ok(())
        }
        Err(err) => {
            diag_file::status(
                "EXPLORER",
                "SPAWN_FALLBACK",
                format!("spawn_err={err}; trying ShellExecuteEx"),
            );
            shell_execute_ex(&exe, None, None)?;
            reassert_dock_after_launch(app);
            Ok(())
        }
    }
}

/// Cold-start an exe via ShellExecuteEx (caller handles "already running").
pub fn launch_exe(app: &AppHandle, app_id: &str) -> Result<(), String> {
    diag_file::status("LAUNCH", "BEGIN", format!("raw_id={app_id}"));

    let resolved = match normalize_launch_path(app_id) {
        Ok(p) => p,
        Err(err) => {
            if is_explorer_path(app_id) {
                return launch_or_activate_explorer(app);
            }
            diag_file::status("LAUNCH", "NORMALIZE_FAIL", &err);
            return Err(err);
        }
    };

    diag_file::status("LAUNCH", "RESOLVED", format!("path={resolved}"));

    if is_explorer_path(&resolved) || is_explorer_path(app_id) {
        return launch_or_activate_explorer(app);
    }

    let launch_path = if !Path::new(&resolved).is_file() {
        if app_id.to_lowercase().ends_with(".lnk") {
            let again = resolve_lnk_target(Path::new(app_id))?;
            diag_file::status("LAUNCH", "LNK_RERESOLVE", format!("→ {again}"));
            again
        } else {
            diag_file::status("LAUNCH", "MISSING", format!("path={resolved}"));
            return Err(format!("path not found: {resolved}"));
        }
    } else {
        resolved
    };

    let parent = Path::new(&launch_path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned());

    shell_execute_ex(&launch_path, None, parent.as_deref())?;
    reassert_dock_after_launch(app);
    Ok(())
}

/// Reveal file in Explorer: `explorer.exe /select,<path>`.
pub fn reveal_in_explorer(app: &AppHandle, app_id: &str) -> Result<(), String> {
    let path = normalize_launch_path(app_id).unwrap_or_else(|_| app_id.to_string());
    let path_obj = Path::new(&path);
    if !path_obj.is_file() {
        diag_file::status("EXPLORER", "REVEAL_MISSING", format!("path={path}"));
        return Err(format!("path not found: {path}"));
    }

    let exe = explorer_exe_path();
    // Always quote; strip embedded quotes so lpParameters cannot break out.
    let safe_path = path.replace('"', "");
    let params = format!("/select,\"{safe_path}\"");
    diag_file::status("EXPLORER", "REVEAL", format!("exe={exe} params={params}"));
    shell_execute_ex(&exe, Some(&params), None)?;
    reassert_dock_after_launch(app);
    Ok(())
}

fn shell_execute_ex(
    file: &str,
    parameters: Option<&str>,
    directory: Option<&str>,
) -> Result<(), String> {
    let file_w: Vec<u16> = Path::new(file)
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect();
    let params_w: Option<Vec<u16>> = parameters.map(|p| p.encode_utf16().chain([0]).collect());
    let dir_w: Option<Vec<u16>> = directory.map(|d| {
        Path::new(d)
            .as_os_str()
            .encode_wide()
            .chain([0])
            .collect()
    });
    let verb_w: Vec<u16> = "open".encode_utf16().chain([0]).collect();

    let mut info: SHELLEXECUTEINFOW = unsafe { zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_FLAG_NO_UI | SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = PCWSTR(verb_w.as_ptr());
    info.lpFile = PCWSTR(file_w.as_ptr());
    info.lpParameters = params_w
        .as_ref()
        .map(|v| PCWSTR(v.as_ptr()))
        .unwrap_or(PCWSTR::null());
    info.lpDirectory = dir_w
        .as_ref()
        .map(|v| PCWSTR(v.as_ptr()))
        .unwrap_or(PCWSTR::null());
    info.nShow = SW_SHOWNORMAL.0 as i32;

    let result = unsafe { ShellExecuteExW(&mut info) };
    let hinst = info.hInstApp.0 as isize;
    if result.is_err() || hinst <= 32 {
        let gle = unsafe { GetLastError().0 };
        diag_file::err(
            "LAUNCH",
            gle,
            format!("ShellExecuteExW file={file:?} params={parameters:?} hInst={hinst}"),
        );
        return Err(format!(
            "ShellExecuteExW failed for {file} hInst={hinst} gle={gle}"
        ));
    }

    let pid = if !info.hProcess.is_invalid() {
        let pid = unsafe { GetProcessId(info.hProcess) };
        let _ = unsafe { CloseHandle(info.hProcess) };
        pid
    } else {
        0
    };

    diag_file::ok(
        "LAUNCH",
        format!(
            "ShellExecuteExW file={file:?} params={parameters:?} hInst={hinst} pid={pid} fg={:?}",
            unsafe { GetForegroundWindow() }
        ),
    );
    Ok(())
}

fn reassert_dock_after_launch(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        ChromeGuard::on_surface_changed(&window);
        diag_file::ok("CHROME", "post-launch on_surface_changed");
    }
}
