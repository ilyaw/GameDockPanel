//! Windows platform implementation for GameDockPanel.

mod apps;
mod chrome;
mod diag_file;
mod icons;
mod input;
mod launch;
mod lifecycle;
mod region;
pub mod seed;
mod window;

pub use seed::is_app_installed;
pub use icons::resolve_app_icon;
pub use apps::{
    activate_or_launch_app, is_bundle_running, quit_app, refresh_dock_icons,
    reveal_app_in_finder, resolve_bundle_id_from_path, start_apps_monitoring,
    zoom_app_above_dock,
};
pub use lifecycle::{setup_dock_window, show_main_window};
pub use diag_file::log_frontend_error;
pub use region::refresh as refresh_dock_backdrop_clip;
pub use window::{
    apply_dock_window_layer, clear_dock_menu_region_hold, ensure_window_fits_menu_overlay,
    log_display_snapshot, log_windows_diag_snapshot, set_dock_region_relaxed,
    shrink_dock_window_to_stored_pill, store_frontend_render_metrics, sync_vibrancy_pill_from_web,
    windows_backdrop_snapshot,
};
pub(crate) use window::{
    log_implausible_pill_chrome, reassert_frameless_chrome_keep_size, set_dock_outer_frame,
};
