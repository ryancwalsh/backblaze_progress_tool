// Backblaze Progress: a read-only viewer for Backblaze Personal Backup's local state.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bz;

use std::path::PathBuf;
use tauri::Manager;

#[tauri::command]
async fn load_snapshot(app: tauri::AppHandle, check_files: bool) -> Result<bz::Snapshot, String> {
    let history_dir = app.path().app_data_dir().ok();
    let dir = bz::default_data_dir();
    tauri::async_runtime::spawn_blocking(move || bz::load(&dir, check_files, history_dir))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn reveal(path: String) -> Result<(), String> {
    bz::reveal(&path).map_err(|e| e.to_string())
}

/// Saves text (CSV export) to the user's Downloads folder and returns the full path.
#[tauri::command]
fn save_text(app: tauri::AppHandle, name: String, contents: String) -> Result<String, String> {
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' { c } else { '_' })
        .collect();
    let dir: PathBuf = app
        .path()
        .download_dir()
        .or_else(|_| app.path().home_dir())
        .map_err(|e| e.to_string())?;
    let p = dir.join(safe);
    // UTF-8 BOM so Excel opens non-ASCII paths correctly
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(contents.as_bytes());
    std::fs::write(&p, bytes).map_err(|e| e.to_string())?;
    Ok(p.display().to_string())
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![load_snapshot, reveal, save_text])
        .run(tauri::generate_context!())
        .expect("error while running Backblaze Progress");
}
