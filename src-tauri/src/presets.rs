use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{
        mpsc::{self, Sender, SyncSender},
        Mutex, OnceLock, RwLock,
    },
    thread,
    time::Duration,
};

use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::{
    audio::{
        create_platform_backend, AudioBackend, AudioCapabilities, AudioDirection, AudioSession,
    },
    config::{export_config_file, import_config_file, AppConfig, ConfigStore, Preset},
};

const AUDIO_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const HUD_WINDOW_LABEL: &str = "hud";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetApplyReport {
    pub applied_streams: usize,
    pub missing_apps: Vec<String>,
    pub errors: Vec<String>,
}

pub fn apply_preset(backend: &mut dyn AudioBackend, preset: &Preset) -> PresetApplyReport {
    let mut report = PresetApplyReport {
        applied_streams: 0,
        missing_apps: Vec::new(),
        errors: Vec::new(),
    };

    if let Some(default_volume) = preset.default_volume {
        match backend.list_sessions(AudioDirection::Output) {
            Ok(sessions) => {
                let mut updated_apps = HashSet::new();
                for session in sessions
                    .into_iter()
                    .filter(|session| session.volume_writable)
                {
                    let app = session
                        .process_name
                        .as_deref()
                        .filter(|name| !name.trim().is_empty())
                        .unwrap_or(&session.app_name);
                    if !updated_apps.insert(app.to_lowercase()) {
                        continue;
                    }
                    match backend.set_app_volume(app, AudioDirection::Output, default_volume) {
                        Ok(updated) => report.applied_streams += updated,
                        Err(error) => report.errors.push(format!("{app}: {error}")),
                    }
                }
            }
            Err(error) => report
                .errors
                .push(format!("could not apply default stream volume: {error}")),
        }
    }

    // Named targets are deliberately applied after the baseline so they always win.
    for target in &preset.targets {
        match backend.set_app_volume(&target.app, AudioDirection::Output, target.volume) {
            Ok(0) => report.missing_apps.push(target.app.clone()),
            Ok(updated) => report.applied_streams += updated,
            Err(error) => report.errors.push(format!("{}: {error}", target.app)),
        }
    }

    report
}

enum AudioCommand {
    ApplyPreset(Preset),
    ListSessions {
        direction: AudioDirection,
        response: SyncSender<Result<Vec<AudioSession>, String>>,
    },
    SetVolume {
        app: String,
        direction: AudioDirection,
        volume_percent: u8,
        response: SyncSender<Result<usize, String>>,
    },
}

#[derive(Clone)]
struct AudioWorker {
    sender: Sender<AudioCommand>,
    capabilities: AudioCapabilities,
}

impl AudioWorker {
    fn start() -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel::<AudioCommand>();
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);

        thread::Builder::new()
            .name("duckd-audio".to_owned())
            .spawn(move || {
                let mut backend = match create_platform_backend() {
                    Ok(backend) => {
                        let capabilities = backend.capabilities();
                        let _ = ready_sender.send(Ok(capabilities));
                        backend
                    }
                    Err(error) => {
                        let _ = ready_sender.send(Err(error.to_string()));
                        return;
                    }
                };

                while let Ok(command) = receiver.recv() {
                    match command {
                        AudioCommand::ApplyPreset(preset) => {
                            let report = apply_preset(backend.as_mut(), &preset);
                            println!(
                                "duckd:preset-applied name={:?} streams={} missing={} errors={}",
                                preset.name,
                                report.applied_streams,
                                report.missing_apps.len(),
                                report.errors.len()
                            );
                            for error in report.errors {
                                eprintln!("duckd: preset {:?}: {error}", preset.name);
                            }
                        }
                        AudioCommand::ListSessions {
                            direction,
                            response,
                        } => {
                            let result = backend
                                .list_sessions(direction)
                                .map_err(|error| error.to_string());
                            let _ = response.send(result);
                        }
                        AudioCommand::SetVolume {
                            app,
                            direction,
                            volume_percent,
                            response,
                        } => {
                            let result = backend
                                .set_app_volume(&app, direction, volume_percent)
                                .map_err(|error| error.to_string());
                            let _ = response.send(result);
                        }
                    }
                }
            })
            .map_err(|error| format!("could not start audio worker: {error}"))?;

        let capabilities = ready_receiver
            .recv()
            .map_err(|error| format!("audio worker stopped during startup: {error}"))??;
        Ok(Self {
            sender,
            capabilities,
        })
    }

    fn apply(&self, preset: Preset) -> Result<(), String> {
        self.sender
            .send(AudioCommand::ApplyPreset(preset))
            .map_err(|error| format!("audio worker is unavailable: {error}"))
    }

    fn list_sessions(&self, direction: AudioDirection) -> Result<Vec<AudioSession>, String> {
        let (response, receiver) = mpsc::sync_channel(1);
        self.sender
            .send(AudioCommand::ListSessions {
                direction,
                response,
            })
            .map_err(|error| format!("audio worker is unavailable: {error}"))?;
        receiver
            .recv_timeout(AUDIO_REQUEST_TIMEOUT)
            .map_err(|error| format!("audio backend did not respond: {error}"))?
    }

    fn set_volume(
        &self,
        app: String,
        direction: AudioDirection,
        volume_percent: u8,
    ) -> Result<usize, String> {
        let (response, receiver) = mpsc::sync_channel(1);
        self.sender
            .send(AudioCommand::SetVolume {
                app,
                direction,
                volume_percent,
                response,
            })
            .map_err(|error| format!("audio worker is unavailable: {error}"))?;
        receiver
            .recv_timeout(AUDIO_REQUEST_TIMEOUT)
            .map_err(|error| format!("audio backend did not respond: {error}"))?
    }
}

#[derive(Clone)]
enum HotkeyAction {
    ToggleHud,
    ApplyPreset(Preset),
}

pub struct PresetRuntime {
    config: ConfigStore,
    audio: AudioWorker,
    registered_shortcuts: Mutex<Vec<Shortcut>>,
    routes: RwLock<HashMap<u32, HotkeyAction>>,
    edit_lock: Mutex<()>,
}

#[derive(Default)]
pub struct PresetRuntimeState {
    runtime: OnceLock<PresetRuntime>,
}

impl PresetRuntimeState {
    fn runtime(&self) -> Result<&PresetRuntime, String> {
        self.runtime
            .get()
            .ok_or_else(|| "duckd backend is still initializing".to_owned())
    }

    pub fn run_in_tray(&self) -> bool {
        self.runtime()
            .map(PresetRuntime::run_in_tray)
            .unwrap_or(true)
    }
}

impl PresetRuntime {
    pub fn initialize<R: Runtime>(app: &tauri::App<R>, config: ConfigStore) -> Result<(), String> {
        let audio = AudioWorker::start()?;
        let initial_config = config.get().map_err(|error| error.to_string())?;
        let runtime = Self {
            config,
            audio,
            registered_shortcuts: Mutex::new(Vec::new()),
            routes: RwLock::new(HashMap::new()),
            edit_lock: Mutex::new(()),
        };

        let state = app.state::<PresetRuntimeState>();
        if state.runtime.set(runtime).is_err() {
            return Err("preset runtime was already initialized".to_owned());
        }
        replace_hotkeys(&app.handle().clone(), state.runtime()?, &initial_config)?;
        Ok(())
    }

    pub fn run_in_tray(&self) -> bool {
        self.config
            .get()
            .map(|config| config.general.run_in_tray)
            .unwrap_or(true)
    }
}

fn parsed_routes(config: &AppConfig) -> Result<Vec<(Shortcut, HotkeyAction)>, String> {
    config.validate().map_err(|error| error.to_string())?;
    let hud_shortcut = config
        .general
        .hud_hotkey
        .parse::<Shortcut>()
        .map_err(|error| {
            format!(
                "invalid HUD hotkey {:?}: {error}",
                config.general.hud_hotkey
            )
        })?;
    let mut ids = HashMap::new();
    ids.insert(hud_shortcut.id(), "HUD".to_owned());

    let mut routes = Vec::with_capacity(config.presets.len() + 1);
    routes.push((hud_shortcut, HotkeyAction::ToggleHud));
    for preset in &config.presets {
        let shortcut = preset.hotkey.parse::<Shortcut>().map_err(|error| {
            format!(
                "invalid hotkey {:?} for preset {:?}: {error}",
                preset.hotkey, preset.name
            )
        })?;
        if let Some(existing) = ids.insert(shortcut.id(), preset.name.clone()) {
            return Err(format!(
                "hotkey {:?} for preset {:?} conflicts with {existing}",
                preset.hotkey, preset.name
            ));
        }
        routes.push((shortcut, HotkeyAction::ApplyPreset(preset.clone())));
    }
    Ok(routes)
}

fn register_one<R: Runtime>(app: &AppHandle<R>, shortcut: Shortcut) -> Result<(), String> {
    app.global_shortcut()
        .on_shortcut(shortcut, |app, shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }

            let managed_state = app.state::<PresetRuntimeState>();
            let state = match managed_state.runtime() {
                Ok(state) => state,
                Err(error) => {
                    eprintln!("duckd: ignored hotkey while starting: {error}");
                    return;
                }
            };
            let action = match state.routes.read() {
                Ok(routes) => routes.get(&shortcut.id()).cloned(),
                Err(_) => {
                    eprintln!("duckd: preset route lock was poisoned");
                    return;
                }
            };

            match action {
                Some(HotkeyAction::ToggleHud) => {
                    if let Err(error) = toggle_hud(app) {
                        eprintln!("duckd: could not toggle HUD: {error}");
                    }
                }
                Some(HotkeyAction::ApplyPreset(preset)) => {
                    println!(
                        "duckd:preset-hotkey-fired name={:?} hotkey={}",
                        preset.name, preset.hotkey
                    );
                    if let Err(error) = state.audio.apply(preset) {
                        eprintln!("duckd: could not queue preset: {error}");
                    }
                }
                None => {}
            }
        })
        .map_err(|error| error.to_string())
}

fn unregister_shortcuts<R: Runtime>(
    app: &AppHandle<R>,
    shortcuts: &[Shortcut],
) -> Result<(), String> {
    if shortcuts.is_empty() {
        return Ok(());
    }
    app.global_shortcut()
        .unregister_multiple(shortcuts.iter().copied())
        .map_err(|error| error.to_string())
}

fn install_routes<R: Runtime>(
    app: &AppHandle<R>,
    routes: &[(Shortcut, HotkeyAction)],
) -> Result<Vec<Shortcut>, String> {
    let mut registered = Vec::with_capacity(routes.len());
    for (shortcut, _) in routes {
        if let Err(error) = register_one(app, *shortcut) {
            if let Err(cleanup_error) = unregister_shortcuts(app, &registered) {
                return Err(format!(
                    "{error}; additionally failed to clean up partial registration: {cleanup_error}"
                ));
            }
            return Err(error);
        }
        registered.push(*shortcut);
    }
    Ok(registered)
}

fn toggle_hud<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let window = app
        .get_webview_window(HUD_WINDOW_LABEL)
        .ok_or_else(|| "HUD window is unavailable".to_owned())?;
    if window.is_visible().map_err(|error| error.to_string())? {
        window.hide().map_err(|error| error.to_string())?;
    } else {
        if let Some(monitor) = window
            .current_monitor()
            .map_err(|error| error.to_string())?
        {
            let scale = monitor.scale_factor();
            let monitor_position = monitor.position().to_logical::<f64>(scale);
            let monitor_size = monitor.size().to_logical::<f64>(scale);
            let window_size = window
                .outer_size()
                .map_err(|error| error.to_string())?
                .to_logical::<f64>(scale);
            let x = monitor_position.x + ((monitor_size.width - window_size.width) / 2.0);
            let y = monitor_position.y + 36.0;
            window
                .set_position(tauri::LogicalPosition::new(x, y))
                .map_err(|error| error.to_string())?;
        }
        window.show().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn replace_hotkeys<R: Runtime>(
    app: &AppHandle<R>,
    state: &PresetRuntime,
    config: &AppConfig,
) -> Result<(), String> {
    let next_routes = parsed_routes(config)?;
    let mut registered = state
        .registered_shortcuts
        .lock()
        .map_err(|_| "registered shortcut lock was poisoned".to_owned())?;
    let previous_shortcuts = registered.clone();
    let previous_routes = state
        .routes
        .read()
        .map_err(|_| "preset route lock was poisoned".to_owned())?
        .clone();

    unregister_shortcuts(app, &previous_shortcuts)?;
    state
        .routes
        .write()
        .map_err(|_| "preset route lock was poisoned".to_owned())?
        .clear();

    match install_routes(app, &next_routes) {
        Ok(next_shortcuts) => {
            *state
                .routes
                .write()
                .map_err(|_| "preset route lock was poisoned".to_owned())? = next_routes
                .into_iter()
                .map(|(shortcut, action)| (shortcut.id(), action))
                .collect();
            *registered = next_shortcuts;
            Ok(())
        }
        Err(error) => {
            let rollback_routes: Vec<_> = previous_shortcuts
                .iter()
                .filter_map(|shortcut| {
                    previous_routes
                        .get(&shortcut.id())
                        .cloned()
                        .map(|action| (*shortcut, action))
                })
                .collect();
            *state
                .routes
                .write()
                .map_err(|_| "preset route lock was poisoned".to_owned())? = previous_routes;
            match install_routes(app, &rollback_routes) {
                Ok(restored) => {
                    *registered = restored;
                    Err(error)
                }
                Err(rollback_error) => {
                    registered.clear();
                    Err(format!(
                        "{error}; additionally failed to restore previous hotkeys: {rollback_error}"
                    ))
                }
            }
        }
    }
}

#[tauri::command]
pub fn get_config(state: State<'_, PresetRuntimeState>) -> Result<AppConfig, String> {
    state
        .runtime()?
        .config
        .get()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_config_path(state: State<'_, PresetRuntimeState>) -> Result<String, String> {
    Ok(state.runtime()?.config.path().display().to_string())
}

#[tauri::command]
pub fn get_audio_capabilities(
    state: State<'_, PresetRuntimeState>,
) -> Result<AudioCapabilities, String> {
    Ok(state.runtime()?.audio.capabilities)
}

#[tauri::command]
pub async fn list_running_processes() -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(crate::processes::list_running_processes)
        .await
        .map_err(|error| format!("process enumeration task failed: {error}"))?
}

#[tauri::command]
pub async fn list_audio_sessions(
    state: State<'_, PresetRuntimeState>,
    direction: AudioDirection,
) -> Result<Vec<AudioSession>, String> {
    let audio = state.runtime()?.audio.clone();
    tauri::async_runtime::spawn_blocking(move || audio.list_sessions(direction))
        .await
        .map_err(|error| format!("audio request task failed: {error}"))?
}

#[tauri::command]
pub async fn set_application_volume(
    state: State<'_, PresetRuntimeState>,
    app: String,
    direction: AudioDirection,
    volume_percent: u8,
) -> Result<usize, String> {
    let audio = state.runtime()?.audio.clone();
    tauri::async_runtime::spawn_blocking(move || audio.set_volume(app, direction, volume_percent))
        .await
        .map_err(|error| format!("audio request task failed: {error}"))?
}

fn replace_runtime_config(
    app: &AppHandle,
    state: &PresetRuntime,
    config: AppConfig,
) -> Result<(), String> {
    let _edit = state
        .edit_lock
        .lock()
        .map_err(|_| "config edit lock was poisoned".to_owned())?;
    let previous = state.config.get().map_err(|error| error.to_string())?;
    replace_hotkeys(app, state, &config)?;

    if let Err(error) = state.config.replace(config) {
        let rollback = replace_hotkeys(app, state, &previous);
        return match rollback {
            Ok(()) => Err(error.to_string()),
            Err(rollback_error) => Err(format!(
                "{error}; additionally failed to restore previous hotkeys: {rollback_error}"
            )),
        };
    }

    Ok(())
}

#[tauri::command]
pub fn save_config(
    app: AppHandle,
    state: State<'_, PresetRuntimeState>,
    config: AppConfig,
) -> Result<(), String> {
    replace_runtime_config(&app, state.runtime()?, config)
}

#[tauri::command]
pub fn import_config(
    app: AppHandle,
    state: State<'_, PresetRuntimeState>,
    path: String,
) -> Result<AppConfig, String> {
    let config = import_config_file(Path::new(&path)).map_err(|error| error.to_string())?;
    replace_runtime_config(&app, state.runtime()?, config.clone())?;
    Ok(config)
}

#[tauri::command]
pub fn export_config(state: State<'_, PresetRuntimeState>, path: String) -> Result<(), String> {
    let config = state
        .runtime()?
        .config
        .get()
        .map_err(|error| error.to_string())?;
    export_config_file(Path::new(&path), &config).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use crate::{
        audio::{
            AudioBackend, AudioCapabilities, AudioDirection, AudioError, AudioResult, AudioSession,
        },
        config::{AppConfig, Preset, PresetTarget},
    };

    use super::{apply_preset, parsed_routes, HotkeyAction, PresetRuntimeState};

    #[derive(Default)]
    struct FakeBackend {
        calls: Vec<(String, AudioDirection, u8)>,
        sessions: Vec<AudioSession>,
    }

    impl AudioBackend for FakeBackend {
        fn capabilities(&self) -> AudioCapabilities {
            AudioCapabilities {
                application_output: true,
                application_input: false,
            }
        }

        fn list_sessions(&mut self, _direction: AudioDirection) -> AudioResult<Vec<AudioSession>> {
            Ok(self.sessions.clone())
        }

        fn set_app_volume(
            &mut self,
            app: &str,
            direction: AudioDirection,
            volume_percent: u8,
        ) -> AudioResult<usize> {
            self.calls.push((app.to_owned(), direction, volume_percent));
            match app {
                "missing" => Ok(0),
                "broken" => Err(AudioError::Backend("test failure".to_owned())),
                _ => Ok(1),
            }
        }
    }

    #[test]
    fn startup_state_reports_initialization_without_panicking() {
        let state = PresetRuntimeState::default();

        assert_eq!(
            state.runtime().err().as_deref(),
            Some("duckd backend is still initializing")
        );
    }

    #[test]
    fn applying_a_preset_updates_running_apps_and_skips_missing_ones() {
        let preset = Preset {
            name: "Test".to_owned(),
            hotkey: "Shift+F3".to_owned(),
            default_volume: None,
            targets: vec![
                PresetTarget {
                    app: "running".to_owned(),
                    volume: 80,
                },
                PresetTarget {
                    app: "missing".to_owned(),
                    volume: 20,
                },
                PresetTarget {
                    app: "broken".to_owned(),
                    volume: 50,
                },
            ],
        };
        let mut backend = FakeBackend::default();

        let report = apply_preset(&mut backend, &preset);

        assert_eq!(report.applied_streams, 1);
        assert_eq!(report.missing_apps, vec!["missing"]);
        assert_eq!(report.errors.len(), 1);
        assert!(backend
            .calls
            .iter()
            .all(|(_, direction, _)| *direction == AudioDirection::Output));
    }

    #[test]
    fn default_volume_is_applied_first_and_named_targets_override_it() {
        let session = |id: &str, app_name: &str, process_name: &str| AudioSession {
            id: id.to_owned(),
            app_name: app_name.to_owned(),
            process_name: Some(process_name.to_owned()),
            direction: AudioDirection::Output,
            volume_percent: 100.0,
            muted: false,
            volume_writable: true,
        };
        let mut backend = FakeBackend {
            calls: Vec::new(),
            sessions: vec![
                session("game-1", "Game", "game.exe"),
                session("discord", "Discord", "discord.exe"),
                session("game-2", "Game", "game.exe"),
            ],
        };
        let preset = Preset {
            name: "Ducked voice chat".to_owned(),
            hotkey: "Shift+F4".to_owned(),
            default_volume: Some(60),
            targets: vec![PresetTarget {
                app: "discord.exe".to_owned(),
                volume: 25,
            }],
        };

        let report = apply_preset(&mut backend, &preset);

        assert!(report.errors.is_empty());
        assert_eq!(
            backend.calls,
            vec![
                ("game.exe".to_owned(), AudioDirection::Output, 60),
                ("discord.exe".to_owned(), AudioDirection::Output, 60),
                ("discord.exe".to_owned(), AudioDirection::Output, 25),
            ]
        );
    }

    #[test]
    fn duplicate_and_hud_conflicting_hotkeys_are_rejected() {
        let mut duplicate = AppConfig::default();
        duplicate.presets[1].hotkey = duplicate.presets[0].hotkey.clone();
        assert!(parsed_routes(&duplicate).is_err());

        duplicate.presets[1].hotkey = duplicate.general.hud_hotkey.clone();
        assert!(parsed_routes(&duplicate).is_err());
    }

    #[test]
    fn default_routes_include_the_hud_and_every_preset() {
        let config = AppConfig::default();
        let routes = parsed_routes(&config).expect("parse default hotkeys");

        assert_eq!(routes.len(), config.presets.len() + 1);
        assert!(matches!(&routes[0].1, HotkeyAction::ToggleHud));
    }
}
