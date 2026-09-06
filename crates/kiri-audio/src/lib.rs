use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("WASAPI is only available on Windows")]
    Unsupported,
    #[error("audio device failed: {0}")]
    Device(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AudioSourceKind {
    Microphone,
    SystemLoopback,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub kind: AudioSourceKind,
    pub is_default: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioMeter {
    pub peak: f32,
    pub clipped: bool,
}

impl AudioMeter {
    #[must_use]
    pub fn from_f32(samples: &[f32]) -> Self {
        let peak = samples
            .iter()
            .fold(0.0_f32, |value, sample| value.max(sample.abs()));
        Self {
            peak: peak.min(1.0),
            clipped: peak >= 0.999,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriftMeasurement {
    pub expected_micros: i64,
    pub observed_micros: i64,
    pub drift_micros: i64,
    pub correction_ratio: f64,
}

#[must_use]
pub fn measure_drift(
    sample_frames: u64,
    sample_rate: u32,
    observed_micros: i64,
) -> DriftMeasurement {
    let expected_micros = if sample_rate == 0 {
        0
    } else {
        (u128::from(sample_frames) * 1_000_000 / u128::from(sample_rate)).min(i64::MAX as u128)
            as i64
    };
    let drift_micros = observed_micros - expected_micros;
    let correction_ratio = if observed_micros <= 0 {
        1.0
    } else {
        (expected_micros as f64 / observed_micros as f64).clamp(0.995, 1.005)
    };
    DriftMeasurement {
        expected_micros,
        observed_micros,
        drift_micros,
        correction_ratio,
    }
}

#[cfg(windows)]
pub fn enumerate_devices() -> Result<Vec<AudioDevice>, AudioError> {
    use wasapi::{DeviceEnumerator, Direction, initialize_mta};
    initialize_mta()
        .ok()
        .map_err(|e| AudioError::Device(e.to_string()))?;
    let enumerator = DeviceEnumerator::new().map_err(|e| AudioError::Device(e.to_string()))?;
    let default_capture = enumerator
        .get_default_device(&Direction::Capture)
        .ok()
        .and_then(|device| device.get_id().ok());
    let default_render = enumerator
        .get_default_device(&Direction::Render)
        .ok()
        .and_then(|device| device.get_id().ok());
    let mut result = Vec::new();
    for (direction, kind, default_id) in [
        (
            Direction::Capture,
            AudioSourceKind::Microphone,
            default_capture,
        ),
        (
            Direction::Render,
            AudioSourceKind::SystemLoopback,
            default_render,
        ),
    ] {
        let collection = enumerator
            .get_device_collection(&direction)
            .map_err(|e| AudioError::Device(e.to_string()))?;
        for device in &collection {
            let device = device.map_err(|e| AudioError::Device(e.to_string()))?;
            let id = device
                .get_id()
                .map_err(|e| AudioError::Device(e.to_string()))?;
            result.push(AudioDevice {
                is_default: default_id.as_deref() == Some(id.as_str()),
                name: device
                    .get_friendlyname()
                    .unwrap_or_else(|_| "Audio device".into()),
                id,
                kind,
            });
        }
    }
    Ok(result)
}

#[cfg(windows)]
pub fn endpoint_meter(
    kind: AudioSourceKind,
    device_id: Option<&str>,
) -> Result<AudioMeter, AudioError> {
    use wasapi::{DeviceEnumerator, Direction, initialize_mta};
    initialize_mta()
        .ok()
        .map_err(|error| AudioError::Device(error.to_string()))?;
    let enumerator =
        DeviceEnumerator::new().map_err(|error| AudioError::Device(error.to_string()))?;
    let direction = match kind {
        AudioSourceKind::Microphone => Direction::Capture,
        AudioSourceKind::SystemLoopback => Direction::Render,
    };
    let device = match device_id {
        Some(id) => enumerator.get_device(id),
        None => enumerator.get_default_device(&direction),
    }
    .map_err(|error| AudioError::Device(error.to_string()))?;
    let peak = device
        .get_audiometerinformation()
        .and_then(|meter| meter.get_peak_value())
        .map_err(|error| AudioError::Device(error.to_string()))?;
    Ok(AudioMeter {
        peak: peak.clamp(0.0, 1.0),
        clipped: peak >= 0.999,
    })
}

#[cfg(not(windows))]
pub fn endpoint_meter(_: AudioSourceKind, _: Option<&str>) -> Result<AudioMeter, AudioError> {
    Err(AudioError::Unsupported)
}

#[cfg(not(windows))]
pub fn enumerate_devices() -> Result<Vec<AudioDevice>, AudioError> {
    Err(AudioError::Unsupported)
}

pub struct AudioRecordingHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<u64, AudioError>>>,
}

impl AudioRecordingHandle {
    pub fn stop(mut self) -> Result<u64, AudioError> {
        self.stop.store(true, Ordering::Release);
        self.thread
            .take()
            .expect("audio thread exists")
            .join()
            .map_err(|_| AudioError::Device("audio capture thread panicked".into()))?
    }
}

#[cfg(windows)]
pub fn start_recording(
    kind: AudioSourceKind,
    device_id: Option<String>,
    path: PathBuf,
) -> Result<AudioRecordingHandle, AudioError> {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(0);
    let thread = std::thread::Builder::new()
        .name(format!("kiri-audio-{kind:?}"))
        .spawn(move || capture_wav(kind, device_id, path, thread_stop, ready_tx))
        .map_err(|e| AudioError::Device(e.to_string()))?;
    if ready_rx.recv().is_err() {
        return Err(thread
            .join()
            .map_err(|_| AudioError::Device("audio capture thread panicked".into()))?
            .err()
            .unwrap_or_else(|| AudioError::Device("audio capture ended before ready".into())));
    }
    Ok(AudioRecordingHandle {
        stop,
        thread: Some(thread),
    })
}

#[cfg(not(windows))]
pub fn start_recording(
    _: AudioSourceKind,
    _: Option<String>,
    _: PathBuf,
) -> Result<AudioRecordingHandle, AudioError> {
    Err(AudioError::Unsupported)
}

#[cfg(windows)]
fn capture_wav(
    kind: AudioSourceKind,
    device_id: Option<String>,
    path: PathBuf,
    stop: Arc<AtomicBool>,
    ready: std::sync::mpsc::SyncSender<()>,
) -> Result<u64, AudioError> {
    use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat, initialize_mta};
    initialize_mta()
        .ok()
        .map_err(|e| AudioError::Device(e.to_string()))?;
    let enumerator = DeviceEnumerator::new().map_err(|e| AudioError::Device(e.to_string()))?;
    let endpoint_direction = match kind {
        AudioSourceKind::Microphone => Direction::Capture,
        AudioSourceKind::SystemLoopback => Direction::Render,
    };
    let device = match device_id {
        Some(id) => enumerator.get_device(&id),
        None => enumerator.get_default_device(&endpoint_direction),
    }
    .map_err(|e| AudioError::Device(e.to_string()))?;
    let mut client = device
        .get_iaudioclient()
        .map_err(|e| AudioError::Device(e.to_string()))?;
    let format = WaveFormat::new(32, 32, &SampleType::Float, 48_000, 2, None);
    let (_, minimum_period) = client
        .get_device_period()
        .map_err(|e| AudioError::Device(e.to_string()))?;
    client
        .initialize_client(
            &format,
            &Direction::Capture,
            &StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: minimum_period,
            },
        )
        .map_err(|e| AudioError::Device(e.to_string()))?;
    let event = client
        .set_get_eventhandle()
        .map_err(|e| AudioError::Device(e.to_string()))?;
    let capture = client
        .get_audiocaptureclient()
        .map_err(|e| AudioError::Device(e.to_string()))?;
    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .map_err(|e| AudioError::Device(e.to_string()))?;
    let mut bytes = VecDeque::new();
    let mut samples = 0_u64;
    client
        .start_stream()
        .map_err(|e| AudioError::Device(e.to_string()))?;
    let _ = ready.send(());
    while !stop.load(Ordering::Acquire) {
        capture
            .read_from_device_to_deque(&mut bytes)
            .map_err(|e| AudioError::Device(e.to_string()))?;
        while bytes.len() >= 4 {
            let sample = f32::from_le_bytes([
                bytes.pop_front().expect("length checked"),
                bytes.pop_front().expect("length checked"),
                bytes.pop_front().expect("length checked"),
                bytes.pop_front().expect("length checked"),
            ]);
            writer
                .write_sample(sample)
                .map_err(|e| AudioError::Device(e.to_string()))?;
            samples += 1;
        }
        let _ = event.wait_for_event(100);
    }
    client
        .stop_stream()
        .map_err(|e| AudioError::Device(e.to_string()))?;
    writer
        .finalize()
        .map_err(|e| AudioError::Device(e.to_string()))?;
    Ok(samples / 2)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn meter_reports_peak_and_clipping() {
        assert_eq!(
            AudioMeter::from_f32(&[0.2, -1.1, 0.4]),
            AudioMeter {
                peak: 1.0,
                clipped: true
            }
        );
    }
    #[test]
    fn drift_policy_is_bounded_and_continuous() {
        let drift = measure_drift(48_000 * 60, 48_000, 60_100_000);
        assert_eq!(drift.expected_micros, 60_000_000);
        assert_eq!(drift.drift_micros, 100_000);
        assert!((0.995..=1.005).contains(&drift.correction_ratio));
    }
}
