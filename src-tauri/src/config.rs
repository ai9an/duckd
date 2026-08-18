use std::{
    error::Error,
    fmt, fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        RwLock,
    },
};

use serde::{Deserialize, Serialize};

static SAVE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub general: GeneralConfig,
    pub presets: Vec<Preset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    pub run_in_tray: bool,
    pub hud_hotkey: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub hotkey: String,
    /// Baseline applied to every active output stream before named targets override it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_volume: Option<u8>,
    #[serde(default)]
    pub targets: Vec<PresetTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresetTarget {
    pub app: String,
    pub volume: u8,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            general: GeneralConfig::default(),
            presets: vec![
                Preset {
                    name: "Focus".to_owned(),
                    hotkey: "Shift+F1".to_owned(),
                    default_volume: None,
                    targets: vec![
                        PresetTarget {
                            app: "Discord".to_owned(),
                            volume: 25,
                        },
                        PresetTarget {
                            app: "MyGame.exe".to_owned(),
                            volume: 100,
                        },
                    ],
                },
                Preset {
                    name: "Chill".to_owned(),
                    hotkey: "Shift+F2".to_owned(),
                    default_volume: None,
                    targets: vec![
                        PresetTarget {
                            app: "MyGame.exe".to_owned(),
                            volume: 40,
                        },
                        PresetTarget {
                            app: "Spotify".to_owned(),
                            volume: 80,
                        },
                    ],
                },
            ],
        }
    }
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            run_in_tray: true,
            hud_hotkey: "Ctrl+Shift+Space".to_owned(),
        }
    }
}

impl AppConfig {
    pub fn validate(&self) -> ConfigResult<()> {
        if self.general.hud_hotkey.trim().is_empty() {
            return Err(ConfigError::Validation(
                "general.hud_hotkey cannot be empty".to_owned(),
            ));
        }

        for (preset_index, preset) in self.presets.iter().enumerate() {
            if preset.name.trim().is_empty() {
                return Err(ConfigError::Validation(format!(
                    "presets[{preset_index}].name cannot be empty"
                )));
            }
            if preset.hotkey.trim().is_empty() {
                return Err(ConfigError::Validation(format!(
                    "preset {:?} has an empty hotkey",
                    preset.name
                )));
            }
            if preset.default_volume.is_some_and(|volume| volume > 100) {
                return Err(ConfigError::Validation(format!(
                    "preset {:?} default volume must be between 0 and 100",
                    preset.name
                )));
            }

            for (target_index, target) in preset.targets.iter().enumerate() {
                if target.app.trim().is_empty() {
                    return Err(ConfigError::Validation(format!(
                        "preset {:?} target {target_index} has an empty app name",
                        preset.name
                    )));
                }
                if target.volume > 100 {
                    return Err(ConfigError::Validation(format!(
                        "preset {:?} target {:?} volume must be between 0 and 100",
                        preset.name, target.app
                    )));
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    Deserialize {
        path: PathBuf,
        source: toml::de::Error,
    },
    Serialize(toml::ser::Error),
    Validation(String),
    LockPoisoned,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "could not {operation} config at {}: {source}",
                path.display()
            ),
            Self::Deserialize { path, source } => {
                write!(formatter, "invalid TOML in {}: {source}", path.display())
            }
            Self::Serialize(source) => write!(formatter, "could not serialize config: {source}"),
            Self::Validation(message) => write!(formatter, "invalid config: {message}"),
            Self::LockPoisoned => write!(formatter, "config state lock was poisoned"),
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Deserialize { source, .. } => Some(source),
            Self::Serialize(source) => Some(source),
            Self::Validation(_) | Self::LockPoisoned => None,
        }
    }
}

pub type ConfigResult<T> = Result<T, ConfigError>;

pub struct ConfigStore {
    path: PathBuf,
    current: RwLock<AppConfig>,
}

impl ConfigStore {
    pub fn load_or_create(path: PathBuf) -> ConfigResult<Self> {
        let config = if path.exists() {
            load_from_path(&path)?
        } else {
            let config = AppConfig::default();
            save_to_path(&path, &config)?;
            config
        };

        config.validate()?;
        Ok(Self {
            path,
            current: RwLock::new(config),
        })
    }

    pub fn get(&self) -> ConfigResult<AppConfig> {
        self.current
            .read()
            .map(|config| config.clone())
            .map_err(|_| ConfigError::LockPoisoned)
    }

    pub fn replace(&self, config: AppConfig) -> ConfigResult<()> {
        config.validate()?;
        save_to_path(&self.path, &config)?;
        *self
            .current
            .write()
            .map_err(|_| ConfigError::LockPoisoned)? = config;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub fn import_config_file(path: &Path) -> ConfigResult<AppConfig> {
    let config = load_from_path(path)?;
    config.validate()?;
    Ok(config)
}

pub fn export_config_file(path: &Path, config: &AppConfig) -> ConfigResult<()> {
    save_to_path(path, config)
}

fn load_from_path(path: &Path) -> ConfigResult<AppConfig> {
    let contents = fs::read_to_string(path).map_err(|source| ConfigError::Io {
        operation: "read",
        path: path.to_owned(),
        source,
    })?;
    toml::from_str(&contents).map_err(|source| ConfigError::Deserialize {
        path: path.to_owned(),
        source,
    })
}

fn save_to_path(path: &Path, config: &AppConfig) -> ConfigResult<()> {
    config.validate()?;
    let contents = toml::to_string_pretty(config).map_err(ConfigError::Serialize)?;
    let parent = path.parent().ok_or_else(|| {
        ConfigError::Validation(format!("config path {} has no parent", path.display()))
    })?;
    fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
        operation: "create the parent directory for",
        path: path.to_owned(),
        source,
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            ConfigError::Validation(format!("config path {} has no file name", path.display()))
        })?;
    let sequence = SAVE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary_path = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        sequence
    ));

    let write_result = (|| -> std::io::Result<()> {
        let mut temporary = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary_path)?;
        temporary.write_all(contents.as_bytes())?;
        temporary.sync_all()?;
        drop(temporary);
        replace_config_file(&temporary_path, path)
    })();

    if let Err(source) = write_result {
        let _ = fs::remove_file(&temporary_path);
        return Err(ConfigError::Io {
            operation: "write",
            path: path.to_owned(),
            source,
        });
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn replace_config_file(temporary_path: &Path, path: &Path) -> std::io::Result<()> {
    fs::rename(temporary_path, path)
}

#[cfg(target_os = "windows")]
fn replace_config_file(temporary_path: &Path, path: &Path) -> std::io::Result<()> {
    use std::{iter, os::windows::ffi::OsStrExt};
    use std::{thread, time::Duration};

    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{ReplaceFileW, REPLACE_FILE_FLAGS},
    };

    if !path.exists() {
        return fs::rename(temporary_path, path);
    }

    let destination: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    let replacement: Vec<u16> = temporary_path
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect();
    const ERROR_SHARING_VIOLATION: i32 = 32;
    const MAX_REPLACE_ATTEMPTS: usize = 7;

    for attempt in 0..MAX_REPLACE_ATTEMPTS {
        // SAFETY: both paths are valid, null-terminated UTF-16 buffers that live through the call.
        let replaced = unsafe {
            ReplaceFileW(
                PCWSTR(destination.as_ptr()),
                PCWSTR(replacement.as_ptr()),
                PCWSTR::null(),
                REPLACE_FILE_FLAGS(0),
                None,
                None,
            )
        };
        if replaced.as_bool() {
            return Ok(());
        }

        let error = std::io::Error::last_os_error();
        let can_retry = error.raw_os_error() == Some(ERROR_SHARING_VIOLATION)
            && attempt + 1 < MAX_REPLACE_ATTEMPTS;
        if !can_retry {
            return Err(error);
        }

        let delay_ms = 20_u64.saturating_mul(1_u64 << attempt.min(3));
        thread::sleep(Duration::from_millis(delay_ms));
    }

    unreachable!("the replacement loop always returns")
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::{export_config_file, import_config_file, AppConfig, ConfigStore};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let unique = format!(
                "duckd-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("system clock")
                    .as_nanos()
            );
            Self(std::env::temp_dir().join(unique))
        }

        fn config_path(&self) -> PathBuf {
            self.0.join("config.toml")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_config_is_created_with_the_documented_defaults() {
        let directory = TestDirectory::new("default-config");
        let path = directory.config_path();
        let store = ConfigStore::load_or_create(path.clone()).expect("create default config");

        assert_eq!(store.get().expect("read store"), AppConfig::default());
        let saved = fs::read_to_string(path).expect("read saved config");
        assert!(saved.contains("hud_hotkey = \"Ctrl+Shift+Space\""));
        assert!(saved.contains("[[presets.targets]]"));
    }

    #[test]
    fn edits_are_saved_and_returned_from_memory() {
        let directory = TestDirectory::new("save-config");
        let path = directory.config_path();
        let store = ConfigStore::load_or_create(path.clone()).expect("create default config");
        let mut edited = store.get().expect("read store");
        edited.presets[0].targets[0].volume = 17;

        store.replace(edited.clone()).expect("save edit");

        assert_eq!(store.get().expect("read edited store"), edited);
        let reloaded = ConfigStore::load_or_create(path)
            .expect("reload config")
            .get()
            .expect("read reloaded config");
        assert_eq!(reloaded, edited);
    }

    #[test]
    fn invalid_volumes_are_rejected() {
        let mut config = AppConfig::default();
        config.presets[0].targets[0].volume = 101;
        assert!(config.validate().is_err());

        config.presets[0].targets[0].volume = 100;
        config.presets[0].default_volume = Some(101);
        assert!(config.validate().is_err());
    }

    #[test]
    fn exported_config_can_be_imported_without_changing_its_shape() {
        let directory = TestDirectory::new("import-export");
        let path = directory.0.join("duckd-export.toml");
        let config = AppConfig::default();

        export_config_file(&path, &config).expect("export config");
        let imported = import_config_file(&path).expect("import config");

        assert_eq!(imported, config);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn save_retries_a_transient_windows_sharing_violation() {
        use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt, thread, time::Duration};

        use windows::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

        let directory = TestDirectory::new("sharing-violation");
        let path = directory.config_path();
        let store = ConfigStore::load_or_create(path.clone()).expect("create default config");
        let blocker = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
            .open(&path)
            .expect("open config without delete sharing");
        let release_blocker = thread::spawn(move || {
            thread::sleep(Duration::from_millis(80));
            drop(blocker);
        });

        let mut edited = store.get().expect("read config");
        edited.general.run_in_tray = !edited.general.run_in_tray;
        store
            .replace(edited.clone())
            .expect("save after sharing lock is released");
        release_blocker.join().expect("release blocker thread");

        assert_eq!(store.get().expect("read edited config"), edited);
    }
}
