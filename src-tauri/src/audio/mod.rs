//! Platform audio backends and the shared interface used by the rest of duckd.

use std::{error::Error, fmt};

use serde::{Deserialize, Serialize};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::LinuxAudioBackend;
#[cfg(target_os = "windows")]
pub use windows::WindowsAudioBackend;

/// Whether an application stream plays audio or captures it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioDirection {
    Output,
    Input,
}

/// Capabilities that genuinely exist on the active platform backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AudioCapabilities {
    pub application_output: bool,
    pub application_input: bool,
}

/// A running per-application audio stream.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AudioSession {
    /// Opaque platform identifier. It must not be persisted in configuration.
    pub id: String,
    /// Human-readable application or stream label.
    pub app_name: String,
    /// Executable/process name when the platform exposes it.
    pub process_name: Option<String>,
    pub direction: AudioDirection,
    /// Average volume across the stream's channels. PulseAudio may report values above 100%.
    pub volume_percent: f32,
    pub muted: bool,
    pub volume_writable: bool,
}

/// Errors produced by a platform audio backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioError {
    Initialization(String),
    Backend(String),
    InvalidVolume(u8),
    Unsupported {
        operation: &'static str,
        platform: &'static str,
    },
}

impl fmt::Display for AudioError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Initialization(message) => {
                write!(formatter, "audio backend initialization failed: {message}")
            }
            Self::Backend(message) => write!(formatter, "audio backend error: {message}"),
            Self::InvalidVolume(volume) => {
                write!(formatter, "volume must be between 0 and 100, got {volume}")
            }
            Self::Unsupported {
                operation,
                platform,
            } => write!(formatter, "{operation} is not supported on {platform}"),
        }
    }
}

impl Error for AudioError {}

pub type AudioResult<T> = Result<T, AudioError>;

/// Shared application-volume interface. Platform implementations remain separate modules.
pub trait AudioBackend {
    fn capabilities(&self) -> AudioCapabilities;

    fn list_sessions(&mut self, direction: AudioDirection) -> AudioResult<Vec<AudioSession>>;

    /// Sets every running stream matching `app` and returns how many streams were updated.
    /// A missing application is a successful no-op and returns zero.
    fn set_app_volume(
        &mut self,
        app: &str,
        direction: AudioDirection,
        volume_percent: u8,
    ) -> AudioResult<usize>;
}

/// Creates the backend for the current supported desktop target.
#[cfg(target_os = "linux")]
pub fn create_platform_backend() -> AudioResult<Box<dyn AudioBackend>> {
    Ok(Box::new(LinuxAudioBackend::new()?))
}

/// Creates the backend for the current supported desktop target.
#[cfg(target_os = "windows")]
pub fn create_platform_backend() -> AudioResult<Box<dyn AudioBackend>> {
    Ok(Box::new(WindowsAudioBackend::new()))
}

pub(crate) fn validate_volume(volume_percent: u8) -> AudioResult<()> {
    if volume_percent <= 100 {
        Ok(())
    } else {
        Err(AudioError::InvalidVolume(volume_percent))
    }
}

pub(crate) fn names_match(target: &str, candidate: &str) -> bool {
    normalize_app_name(target) == normalize_app_name(candidate)
}

fn normalize_app_name(value: &str) -> String {
    let filename = value
        .trim()
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim();
    let lowercase = filename.to_lowercase();
    lowercase
        .strip_suffix(".exe")
        .unwrap_or(&lowercase)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::{names_match, validate_volume, AudioError};

    #[test]
    fn application_matching_is_exact_and_platform_tolerant() {
        assert!(names_match("Discord", "discord.exe"));
        assert!(names_match(
            "C:\\Program Files\\Spotify\\Spotify.exe",
            "spotify"
        ));
        assert!(names_match("/usr/bin/spotify", "Spotify"));
        assert!(!names_match("game", "game-launcher"));
    }

    #[test]
    fn preset_volume_range_is_enforced() {
        assert!(validate_volume(0).is_ok());
        assert!(validate_volume(100).is_ok());
        assert_eq!(validate_volume(101), Err(AudioError::InvalidVolume(101)));
    }
}
