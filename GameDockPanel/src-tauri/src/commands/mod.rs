//! Tauri commands grouped by domain (see dockpanel rule: "команды по домену
//! в `commands/`"). OS-specific mechanics live in `platform/`; this module
//! only owns the cross-platform data shape and command surface.

pub mod apps;
pub mod window;
