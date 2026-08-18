pub mod audio;
pub mod config;
mod processes;

#[cfg(desktop)]
mod desktop;
#[cfg(desktop)]
mod presets;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .manage(presets::PresetRuntimeState::default())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            presets::get_config,
            presets::get_config_path,
            presets::save_config,
            presets::import_config,
            presets::export_config,
            presets::get_audio_capabilities,
            presets::list_running_processes,
            presets::list_audio_sessions,
            presets::set_application_volume
        ])
        .on_window_event(|window, event| match (window.label(), event) {
            ("main", tauri::WindowEvent::CloseRequested { api, .. }) => {
                use tauri::Manager;

                let run_in_tray = window.state::<presets::PresetRuntimeState>().run_in_tray();
                if run_in_tray {
                    api.prevent_close();
                    if let Err(error) = window.hide() {
                        eprintln!("duckd: failed to hide the main window: {error}");
                    }
                } else {
                    api.prevent_close();
                    window.app_handle().exit(0);
                }
            }
            ("hud", tauri::WindowEvent::CloseRequested { api, .. }) => {
                api.prevent_close();
                if let Err(error) = window.hide() {
                    eprintln!("duckd: failed to hide the HUD: {error}");
                }
            }
            ("hud", tauri::WindowEvent::Focused(false)) => {
                if let Err(error) = window.hide() {
                    eprintln!("duckd: failed to dismiss the HUD: {error}");
                }
            }
            _ => {}
        });

    #[cfg(desktop)]
    let builder = builder.setup(desktop::setup);

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
