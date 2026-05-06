use tauri::{LogicalSize, Manager};

#[tauri::command]
fn start_window_drag(window: tauri::Window) -> Result<(), String> {
  window.start_dragging().map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .plugin(tauri_plugin_opener::init())
    .invoke_handler(tauri::generate_handler![start_window_drag])
    .setup(|app| {
      if let Some(window) = app.get_webview_window("main") {
        let app_size = LogicalSize::new(1180.0, 760.0);
        window.set_min_size(Some(app_size))?;
        window.set_size(app_size)?;
      }

      if cfg!(debug_assertions) {
        app.handle().plugin(
          tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .build(),
        )?;
      }
      Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
