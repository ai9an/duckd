use std::{
    any::Any,
    cell::Cell,
    panic::{catch_unwind, UnwindSafe},
    rc::Rc,
};

use libpulse_binding::{
    proplist::properties::{APPLICATION_NAME, APPLICATION_PROCESS_BINARY},
    volume::{Volume, VOLUME_NORM},
};
use pulsectl::controllers::{types::ApplicationInfo, AppControl, SinkController, SourceController};

use super::{
    names_match, validate_volume, AudioBackend, AudioCapabilities, AudioDirection, AudioError,
    AudioResult, AudioSession,
};

/// PipeWire/PulseAudio backend. Output and input use independent pulsectl controllers.
pub struct LinuxAudioBackend {
    output: SinkController,
    input: SourceController,
}

impl LinuxAudioBackend {
    pub fn new() -> AudioResult<Self> {
        let output = create_controller("output", SinkController::create)?;
        let input = create_controller("input", SourceController::create)?;
        Ok(Self { output, input })
    }

    fn list_output_sessions(&mut self) -> AudioResult<Vec<AudioSession>> {
        self.output
            .list_applications()
            .map_err(|error| pulse_error("list sink inputs", error))
            .map(|applications| {
                applications
                    .iter()
                    .map(|application| to_session(application, AudioDirection::Output))
                    .collect()
            })
    }

    fn list_input_sessions(&mut self) -> AudioResult<Vec<AudioSession>> {
        self.input
            .list_applications()
            .map_err(|error| pulse_error("list source outputs", error))
            .map(|applications| {
                applications
                    .iter()
                    .map(|application| to_session(application, AudioDirection::Input))
                    .collect()
            })
    }

    fn set_output_volume(&mut self, app: &str, volume_percent: u8) -> AudioResult<usize> {
        let applications = self
            .output
            .list_applications()
            .map_err(|error| pulse_error("list sink inputs", error))?;
        let mut updated = 0;

        for application in applications
            .into_iter()
            .filter(|application| application_matches(app, application))
        {
            ensure_writable(&application)?;
            let volumes = volumes_at_percent(&application, volume_percent)?;
            let succeeded = Rc::new(Cell::new(false));
            let callback_result = Rc::clone(&succeeded);
            let operation = self.output.handler.introspect.set_sink_input_volume(
                application.index,
                &volumes,
                Some(Box::new(move |result| callback_result.set(result))),
            );
            self.output
                .handler
                .wait_for_operation(operation)
                .map_err(|error| pulse_error("set sink-input volume", error))?;
            ensure_operation_succeeded(succeeded.get(), "set sink-input volume")?;
            updated += 1;
        }

        Ok(updated)
    }

    fn set_input_volume(&mut self, app: &str, volume_percent: u8) -> AudioResult<usize> {
        let applications = self
            .input
            .list_applications()
            .map_err(|error| pulse_error("list source outputs", error))?;
        let mut updated = 0;

        for application in applications
            .into_iter()
            .filter(|application| application_matches(app, application))
        {
            ensure_writable(&application)?;
            let volumes = volumes_at_percent(&application, volume_percent)?;
            let succeeded = Rc::new(Cell::new(false));
            let callback_result = Rc::clone(&succeeded);
            let operation = self.input.handler.introspect.set_source_output_volume(
                application.index,
                &volumes,
                Some(Box::new(move |result| callback_result.set(result))),
            );
            self.input
                .handler
                .wait_for_operation(operation)
                .map_err(|error| pulse_error("set source-output volume", error))?;
            ensure_operation_succeeded(succeeded.get(), "set source-output volume")?;
            updated += 1;
        }

        Ok(updated)
    }
}

impl AudioBackend for LinuxAudioBackend {
    fn capabilities(&self) -> AudioCapabilities {
        AudioCapabilities {
            application_output: true,
            application_input: true,
        }
    }

    fn list_sessions(&mut self, direction: AudioDirection) -> AudioResult<Vec<AudioSession>> {
        match direction {
            AudioDirection::Output => self.list_output_sessions(),
            AudioDirection::Input => self.list_input_sessions(),
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
            AudioDirection::Output => self.set_output_volume(app, volume_percent),
            AudioDirection::Input => self.set_input_volume(app, volume_percent),
        }
    }
}

fn create_controller<T, F>(direction: &str, create: F) -> AudioResult<T>
where
    F: FnOnce() -> T + UnwindSafe,
{
    catch_unwind(create).map_err(|payload| {
        AudioError::Initialization(format!(
            "could not connect the {direction} controller to PipeWire/PulseAudio: {}",
            panic_message(payload)
        ))
    })
}

fn panic_message(payload: Box<dyn Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown pulsectl panic".to_owned()
    }
}

fn to_session(application: &ApplicationInfo, direction: AudioDirection) -> AudioSession {
    let process_name = property(application, APPLICATION_PROCESS_BINARY)
        .or_else(|| property(application, APPLICATION_NAME));
    let app_name = property(application, APPLICATION_NAME)
        .or_else(|| application.name.clone())
        .or_else(|| process_name.clone())
        .unwrap_or_else(|| "Unknown audio stream".to_owned());
    let stream_kind = match direction {
        AudioDirection::Output => "sink-input",
        AudioDirection::Input => "source-output",
    };

    AudioSession {
        id: format!("pulse:{stream_kind}:{}", application.index),
        app_name,
        process_name,
        direction,
        volume_percent: pulse_volume_percent(application.volume.avg()),
        muted: application.mute,
        volume_writable: application.volume_writable,
    }
}

fn application_matches(target: &str, application: &ApplicationInfo) -> bool {
    property(application, APPLICATION_PROCESS_BINARY)
        .into_iter()
        .chain(property(application, APPLICATION_NAME))
        .chain(application.name.clone())
        .any(|candidate| names_match(target, &candidate))
}

fn property(application: &ApplicationInfo, key: &str) -> Option<String> {
    application
        .proplist
        .get_str(key)
        .filter(|value| !value.trim().is_empty())
}

fn pulse_volume_percent(volume: Volume) -> f32 {
    (volume.0 as f32 / VOLUME_NORM.0 as f32) * 100.0
}

fn volumes_at_percent(
    application: &ApplicationInfo,
    volume_percent: u8,
) -> AudioResult<libpulse_binding::volume::ChannelVolumes> {
    let channels = application.volume.len();
    if channels == 0 {
        return Err(AudioError::Backend(format!(
            "stream {} has no audio channels",
            application.index
        )));
    }

    let raw_volume = ((VOLUME_NORM.0 as u64 * u64::from(volume_percent)) / 100) as u32;
    let mut volumes = application.volume;
    volumes.set(u32::from(channels), Volume(raw_volume));
    Ok(volumes)
}

fn ensure_writable(application: &ApplicationInfo) -> AudioResult<()> {
    if application.volume_writable {
        Ok(())
    } else {
        Err(AudioError::Backend(format!(
            "stream {} does not allow volume changes",
            application.index
        )))
    }
}

fn ensure_operation_succeeded(succeeded: bool, operation: &str) -> AudioResult<()> {
    if succeeded {
        Ok(())
    } else {
        Err(AudioError::Backend(format!(
            "PipeWire/PulseAudio rejected operation: {operation}"
        )))
    }
}

fn pulse_error(operation: &str, error: impl std::fmt::Debug) -> AudioError {
    AudioError::Backend(format!("could not {operation}: {error:?}"))
}

#[cfg(test)]
mod tests {
    use super::{pulse_volume_percent, LinuxAudioBackend};
    use crate::audio::{AudioBackend, AudioDirection};
    use libpulse_binding::volume::{VOLUME_MUTED, VOLUME_NORM};

    #[test]
    fn pulse_volume_uses_normal_as_one_hundred_percent() {
        assert_eq!(pulse_volume_percent(VOLUME_MUTED), 0.0);
        assert!((pulse_volume_percent(VOLUME_NORM) - 100.0).abs() < f32::EPSILON);
    }

    #[test]
    #[ignore = "requires a running PipeWire-Pulse or PulseAudio server"]
    fn live_backend_connects_and_lists_both_stream_directions() {
        let mut backend = LinuxAudioBackend::new().expect("connect to PipeWire-Pulse");
        let output = backend
            .list_sessions(AudioDirection::Output)
            .expect("list sink inputs");
        let input = backend
            .list_sessions(AudioDirection::Input)
            .expect("list source outputs");

        eprintln!(
            "live PipeWire-Pulse streams: {} output, {} input",
            output.len(),
            input.len()
        );
        for session in output.iter().chain(input.iter()) {
            eprintln!(
                "  {:?}: app={:?} process={:?} volume={:.2}%",
                session.direction, session.app_name, session.process_name, session.volume_percent
            );
        }
    }
}
