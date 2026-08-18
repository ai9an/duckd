use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    App, AppHandle, Manager, Runtime,
};

use crate::{config::ConfigStore, presets::PresetRuntime};

const MAIN_WINDOW_LABEL: &str = "main";
const TOGGLE_MENU_ID: &str = "toggle-main-window";
const QUIT_MENU_ID: &str = "quit";

pub fn toggle_main_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return Ok(());
    };

    if window.is_visible()? {
        window.hide()?;
    } else {
        window.unminimize()?;
        window.show()?;
        window.set_focus()?;
    }

    Ok(())
}

pub fn setup(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let config_path = app.path().app_config_dir()?.join("config.toml");
    let config = ConfigStore::load_or_create(config_path)?;
    println!("duckd:config-loaded path={}", config.path().display());
    PresetRuntime::initialize(app, config)?;

    let toggle = MenuItem::with_id(app, TOGGLE_MENU_ID, "Show / Hide duckd", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, QUIT_MENU_ID, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &separator, &quit])?;

    let mut tray = TrayIconBuilder::with_id("duckd-tray")
        .tooltip("duckd audio manager")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            TOGGLE_MENU_ID => {
                if let Err(error) = toggle_main_window(app) {
                    eprintln!("duckd: failed to toggle the main window: {error}");
                }
            }
            QUIT_MENU_ID => app.exit(0),
            _ => {}
        });

    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }

    tray.build(app)?;

    println!("duckd:preset-hotkeys-registered");
    Ok(())
}
