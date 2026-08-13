//! Helios desktop shell.
//!
//! The shell is deliberately thin: it owns the window, the menu and the IPC
//! surface, and nothing else. All analysis lives in `helios-core`, which is why
//! the same engine backs the CLI and will back the Windows build unchanged.

// Hides the console window that would otherwise appear behind the app on
// Windows. No effect on macOS.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;

use state::AppState;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::list_volumes,
            commands::start_scan,
            commands::pause_scan,
            commands::resume_scan,
            commands::cancel_scan,
            commands::active_scan,
            commands::scan_summary,
            commands::loaded_scans,
            commands::list_children,
            commands::ancestors,
            commands::treemap_layout,
            commands::largest_entries,
            commands::search_entries,
            commands::category_breakdown,
            commands::scan_issues,
            commands::export_report,
            commands::list_snapshots,
            commands::load_snapshot,
            commands::forget_scan,
            commands::reveal_in_file_manager,
            commands::app_info,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Helios");
}
