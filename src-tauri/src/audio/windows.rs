use std::collections::HashSet;

use windows::Win32::System::Com::CoUninitialize;
use windows_volume_control::{AudioController, CoinitMode};

use super::{
    names_match, validate_volume, AudioBackend, AudioCapabilities, AudioDirection, AudioError,
    AudioResult, AudioSession,
};

/// WASAPI application-session backend powered by windows-volume-control.
pub struct WindowsAudioBackend;

/// Balances the COM initialization performed internally by windows-volume-control.
struct ScopedAudioController {
    controller: Option<AudioController>,
}

impl ScopedAudioController {
    unsafe fn new() -> Self {
        let mut controller = AudioController::init(Some(CoinitMode::ApartmentThreaded));
        controller.GetSessions();
        controller.GetDefaultAudioEnpointVolumeControl();
        controller.GetAllProcessSessions();
        Self {
            controller: Some(controller),
        }
    }

    fn get(&self) -> &AudioController {
        self.controller
            .as_ref()
            .expect("audio controller is present until its guard is dropped")
    }
}

impl Drop for ScopedAudioController {
    fn drop(&mut self) {
        // COM interface pointers must be released before the apartment is uninitialized.
        drop(self.controller.take());
        // SAFETY: windows-volume-control successfully initialized COM on this same thread.
        unsafe { CoUninitialize() };
    }
}

impl WindowsAudioBackend {
    pub fn new() -> Self {
        Self
    }

    fn with_controller<T>(operation: impl FnOnce(&AudioController) -> T) -> T {
        // SAFETY: windows-volume-control exposes its entire API as unsafe. The controller and every
        // borrowed session remain on this thread and are dropped before this function returns.
        unsafe {
            let controller = ScopedAudioController::new();
            operation(controller.get())
        }
    }

    fn list_output_sessions(&self) -> Vec<AudioSession> {
        Self::with_controller(|controller| {
            // SAFETY: the controller and its sessions are valid for this closure's duration.
            unsafe {
                let mut seen = HashSet::new();
                controller
                    .get_all_session_names()
                    .into_iter()
                    .filter(|name| name != "master")
                    .filter(|name| seen.insert(name.to_lowercase()))
                    .filter_map(|name| {
                        let session = controller.get_session_by_name(name.clone())?;
                        Some(AudioSession {
                            id: format!("wasapi:session:{}", name.to_lowercase()),
                            app_name: name.clone(),
                            process_name: Some(name),
                            direction: AudioDirection::Output,
                            volume_percent: session.getVolume() * 100.0,
                            muted: session.getMute(),
                            volume_writable: true,
                        })
                    })
                    .collect()
            }
        })
    }

    fn set_output_volume(&self, app: &str, volume_percent: u8) -> usize {
        Self::with_controller(|controller| {
            // SAFETY: the controller and its sessions are valid for this closure's duration.
            unsafe {
                let mut updated = HashSet::new();

                for name in controller.get_all_session_names() {
                    if name != "master" && names_match(app, &name) && updated.insert(name.clone()) {
                        if let Some(session) = controller.get_session_by_name(name) {
                            session.setVolume(f32::from(volume_percent) / 100.0);
                        }
                    }
                }

                updated.len()
            }
        })
    }
}

impl Default for WindowsAudioBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioBackend for WindowsAudioBackend {
    fn capabilities(&self) -> AudioCapabilities {
        AudioCapabilities {
            application_output: true,
            application_input: false,
        }
    }

    fn list_sessions(&mut self, direction: AudioDirection) -> AudioResult<Vec<AudioSession>> {
        match direction {
            AudioDirection::Output => Ok(self.list_output_sessions()),
            AudioDirection::Input => Err(AudioError::Unsupported {
                operation: "per-application input volume",
                platform: "Windows",
            }),
        }
    }

    fn set_app_volume(
        &mut self,
        app: &str,
        direction: AudioDirection,
        volume_percent: u8,
    ) -> AudioResult<usize> {
        validate_volume(volume_percent)?;
        match direction {
            AudioDirection::Output => Ok(self.set_output_volume(app, volume_percent)),
            AudioDirection::Input => Err(AudioError::Unsupported {
                operation: "per-application input volume",
                platform: "Windows",
            }),
        }
    }
}
