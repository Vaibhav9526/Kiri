use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("WASAPI is only available on Windows")]
    Unsupported,
    #[error("microphone unavailable: {0}")]
    MicrophoneUnavailable(String),
    #[error("system audio loopback unavailable: {0}")]
    LoopbackUnavailable(String),
    #[error("timed out waiting for audio capture to start: {0}")]
    StartTimeout(String),
    #[error("audio device failed: {0}")]
    Device(String),
}

/// How long `start_recording` waits for the capture thread to signal readiness,
/// mirroring Recordly's 12s native-capture start timeout.
pub const AUDIO_START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(12);

/// How long `start_recording` waits for a timed-out capture thread to exit
/// before detaching it. The capture loop polls `stop` every ~100ms, so this
/// is ample for a clean exit while guaranteeing `start_recording` itself
/// never blocks forever when device init hangs.
const START_TIMEOUT_JOIN_GRACE: Duration = Duration::from_secs(5);

/// Peak at or above this level counts as clipped, matching the preview
/// meter behaviour ported from Recordly (`audio-level-meter` / `useAudioLevelMeter`).
const CLIP_THRESHOLD: f32 = 0.999;

/// Bounded tempo-correction range for `measure_drift`, ported from Recordly's
/// audio-sync policy (fixed small correction avoids audible flutter from
/// continuous micro-corrections on system audio).
const DRIFT_RATIO_MIN: f64 = 0.995;
const DRIFT_RATIO_MAX: f64 = 1.005;

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
        // NaN samples carry no level information (ignore them); infinite
        // samples indicate overload (treat as full-scale clip). This keeps
        // downstream UI/serialization free of NaN while preserving clip detection.
        let mut peak = 0.0_f32;
        for sample in samples {
            let magnitude = if sample.is_nan() { 0.0 } else { sample.abs() };
            if magnitude > peak {
                peak = magnitude;
            }
        }
        let sanitized = sanitize_peak(peak);
        Self {
            peak: sanitized,
            clipped: peak >= CLIP_THRESHOLD,
        }
    }
}

/// Clamp a raw WASAPI/meter peak into `[0, 1]`, mapping non-finite input to
/// silence instead of leaking NaN/Inf to the UI or serialized project.
#[must_use]
pub fn sanitize_peak(raw: f32) -> f32 {
    if !raw.is_finite() {
        return 0.0;
    }
    raw.clamp(0.0, 1.0)
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
    // saturating_sub: observed can be i64::MIN in hostile telemetry without
    // panicking in debug builds.
    let drift_micros = observed_micros.saturating_sub(expected_micros);
    // No meaningful ratio when either side carries no time information
    // (empty capture, zero rate, or non-positive wall clock). Return unity so
    // callers apply no tempo correction instead of slamming to a clamp bound.
    let correction_ratio = if expected_micros <= 0 || observed_micros <= 0 {
        1.0
    } else {
        let ratio = expected_micros as f64 / observed_micros as f64;
        if !ratio.is_finite() {
            1.0
        } else {
            ratio.clamp(DRIFT_RATIO_MIN, DRIFT_RATIO_MAX)
        }
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
    // Best-effort COM init: the calling thread (e.g. a Tauri/STA thread) may
    // already be initialized with a different apartment
    // (RPC_E_CHANGED_MODE). The enumerator can still succeed, so never fail
    // closed here — attempt creation regardless, matching Recordly's native
    // code which ignores per-thread CoInitializeEx results.
    let _ = initialize_mta().ok();
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
    // Best-effort COM init (see `enumerate_devices`).
    let _ = initialize_mta().ok();
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
    let raw_peak = device
        .get_audiometerinformation()
        .and_then(|meter| meter.get_peak_value())
        .map_err(|error| AudioError::Device(error.to_string()))?;
    // WASAPI can theoretically report non-finite/negative peaks on failing
    // drivers; sanitize so callers never see NaN.
    let peak = sanitize_peak(raw_peak);
    Ok(AudioMeter {
        peak,
        clipped: peak >= CLIP_THRESHOLD,
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
        let Some(thread) = self.thread.take() else {
            return Err(AudioError::Device(
                "audio capture thread was already stopped".into(),
            ));
        };
        thread
            .join()
            .unwrap_or_else(|_| Err(AudioError::Device("audio capture thread panicked".into())))
    }
}

impl Drop for AudioRecordingHandle {
    fn drop(&mut self) {
        // Best-effort signal so a dropped-but-unstopped handle (e.g. a failed
        // `start_segment` that forgot a path) does not leave WASAPI streaming
        // forever. The detached thread observes the flag and exits.
        self.stop.store(true, Ordering::Release);
    }
}

/// Ensure the parent folder for `path` exists. A bare filename (no parent, or
/// an empty parent such as `Some("")`) means "current directory" and needs no
/// creation — `create_dir_all("")` would otherwise fail spuriously.
fn ensure_parent_dir(kind: AudioSourceKind, path: &Path) -> Result<(), AudioError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(parent)
        .map_err(|e| friendly_error(kind, format!("cannot create output folder: {e}")))?;
    Ok(())
}

/// Preflight check with actionable messages for the Tauri layer. Verifies the
/// requested device still exists before any file is created.
#[cfg(windows)]
pub fn check_device_available(
    kind: AudioSourceKind,
    device_id: Option<&str>,
) -> Result<(), AudioError> {
    let devices = enumerate_devices()?;
    match kind {
        AudioSourceKind::Microphone => match device_id {
            Some(wanted) => {
                if devices
                    .iter()
                    .any(|d| d.kind == AudioSourceKind::Microphone && d.id == wanted)
                {
                    Ok(())
                } else {
                    Err(AudioError::MicrophoneUnavailable(format!(
                        "microphone '{wanted}' was unplugged or disabled; reconnect it or pick another microphone"
                    )))
                }
            }
            // Default device requested: still fail closed when no capture
            // endpoint exists at all, so the UI can warn before recording.
            None => {
                if devices
                    .iter()
                    .any(|d| d.kind == AudioSourceKind::Microphone)
                {
                    Ok(())
                } else {
                    Err(AudioError::MicrophoneUnavailable(
                        "no microphone found; reconnect a microphone or turn microphone capture off"
                            .into(),
                    ))
                }
            }
        },
        AudioSourceKind::SystemLoopback => match device_id {
            Some(wanted) => {
                if devices
                    .iter()
                    .any(|d| d.kind == AudioSourceKind::SystemLoopback && d.id == wanted)
                {
                    Ok(())
                } else {
                    Err(AudioError::LoopbackUnavailable(format!(
                        "system audio device '{wanted}' was unplugged or disabled; check Windows sound settings or turn system audio off"
                    )))
                }
            }
            None => {
                if devices
                    .iter()
                    .any(|d| d.kind == AudioSourceKind::SystemLoopback)
                {
                    Ok(())
                } else {
                    Err(AudioError::LoopbackUnavailable(
                        "no system-audio output device found; check Windows sound settings or turn system audio off"
                            .into(),
                    ))
                }
            }
        },
    }
}

#[cfg(not(windows))]
pub fn check_device_available(_: AudioSourceKind, _: Option<&str>) -> Result<(), AudioError> {
    Err(AudioError::Unsupported)
}

/// Shared helper so unit tests can cover the parent-dir guard without WASAPI.
#[cfg(not(windows))]
fn ensure_parent_dir_fallback(kind: AudioSourceKind, path: &Path) -> Result<(), AudioError> {
    ensure_parent_dir(kind, path)
}

#[cfg(windows)]
pub fn start_recording(
    kind: AudioSourceKind,
    device_id: Option<String>,
    path: PathBuf,
) -> Result<AudioRecordingHandle, AudioError> {
    ensure_parent_dir(kind, &path)?;
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(0);
    let thread = std::thread::Builder::new()
        .name(format!("kiri-audio-{kind:?}"))
        .spawn(move || capture_wav(kind, device_id, path, thread_stop, ready_tx))
        .map_err(|e| AudioError::Device(e.to_string()))?;
    match ready_rx.recv_timeout(AUDIO_START_TIMEOUT) {
        Ok(()) => Ok(AudioRecordingHandle {
            stop,
            thread: Some(thread),
        }),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            stop.store(true, Ordering::Release);
            // Bounded reaping: the capture thread only blocks ~100ms per
            // event-wait iteration once it reaches its loop, but a hang
            // inside device init never observes `stop`. Blocking forever
            // here would defeat the timeout, so detach after a grace period
            // and let the thread finalize on its own.
            let deadline = Instant::now() + START_TIMEOUT_JOIN_GRACE;
            while !thread.is_finished() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            if thread.is_finished() {
                let _ = thread.join();
            }
            Err(AudioError::StartTimeout(format!(
                "{} capture did not start within {}s; the device may be in use by another app",
                match kind {
                    AudioSourceKind::Microphone => "microphone",
                    AudioSourceKind::SystemLoopback => "system audio",
                },
                AUDIO_START_TIMEOUT.as_secs()
            )))
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            let inner = thread
                .join()
                .unwrap_or_else(|_| Err(AudioError::Device("audio capture thread panicked".into())))
                .err()
                .unwrap_or_else(|| friendly_error(kind, "audio capture ended before ready"));
            Err(inner)
        }
    }
}

fn friendly_error(kind: AudioSourceKind, message: impl Into<String>) -> AudioError {
    let message = message.into();
    match kind {
        AudioSourceKind::Microphone => AudioError::MicrophoneUnavailable(message),
        AudioSourceKind::SystemLoopback => AudioError::LoopbackUnavailable(message),
    }
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
    // Best-effort COM init (see `enumerate_devices`).
    let _ = initialize_mta().ok();
    let enumerator = DeviceEnumerator::new().map_err(|e| AudioError::Device(e.to_string()))?;
    let endpoint_direction = match kind {
        AudioSourceKind::Microphone => Direction::Capture,
        AudioSourceKind::SystemLoopback => Direction::Render,
    };
    let device = match device_id.clone() {
        Some(id) => enumerator.get_device(&id),
        None => enumerator.get_default_device(&endpoint_direction),
    }
    .map_err(|e| {
        friendly_error(
            kind,
            match kind {
                AudioSourceKind::Microphone => format!(
                    "microphone '{}' could not be opened ({e}); it may have been unplugged",
                    device_id.as_deref().unwrap_or("default")
                ),
                AudioSourceKind::SystemLoopback => format!(
                    "system audio loopback could not be opened ({e}); check Windows sound settings"
                ),
            },
        )
    })?;
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
        &path,
        hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .map_err(|e| AudioError::Device(e.to_string()))?;
    // Independent 48kHz stereo float tracks for mic + loopback keep drift
    // measurement (`measure_drift`) meaningful across sources.
    const BYTES_PER_FRAME: usize = 2 * 4;
    /// Upper bound for queued-but-unwritten bytes (~10s of stereo float).
    /// Guards against unbounded RAM if the disk stalls while WASAPI keeps
    /// producing packets.
    const MAX_QUEUED_BYTES: usize = 48_000 * BYTES_PER_FRAME * 10;
    if let Err(e) = client.start_stream() {
        // Finalize the empty file so the segment stays a valid (silent) WAV
        // for recovery instead of a corrupt placeholder.
        let _ = writer.finalize();
        return Err(AudioError::Device(e.to_string()));
    }
    let _ = ready.send(());
    let mut bytes = VecDeque::new();
    let mut samples = 0_u64;
    let disconnect_error = |detail: String| {
        friendly_error(
            kind,
            match kind {
                AudioSourceKind::Microphone => format!(
                    "microphone disconnected during recording ({detail}); audio after this point is missing"
                ),
                AudioSourceKind::SystemLoopback => {
                    format!("system audio device disconnected during recording ({detail})")
                }
            },
        )
    };
    // Run the loop inside a closure so the WAV writer below is *always*
    // finalized — including write failures and disconnects — keeping partial
    // segments playable for crash recovery (PRD REC-014).
    let capture_result: Result<u64, AudioError> = (|| {
        while !stop.load(Ordering::Acquire) {
            // Drain every queued packet (Recordly's wasapi_loopback drains via
            // GetNextPacketSize). Reading a single packet per 100ms wait would
            // fall behind under load and manifest as growing A/V drift.
            loop {
                let pending_frames: u32 = match capture.get_next_packet_size() {
                    Ok(Some(n)) if n > 0 => n,
                    Ok(_) => break,
                    Err(e) => return Err(disconnect_error(e.to_string())),
                };
                let before = bytes.len();
                let info = match capture.read_from_device_to_deque(&mut bytes) {
                    Ok(info) => info,
                    Err(e) => return Err(disconnect_error(e.to_string())),
                };
                if info.flags.silent {
                    // WASAPI silent packets (idle loopback): payload bytes are
                    // undefined — replace with zeros so the timeline stays
                    // aligned instead of persisting driver garbage.
                    bytes.truncate(before);
                    let expected =
                        (pending_frames as usize).saturating_mul(BYTES_PER_FRAME);
                    let zeros = if expected > 0 {
                        expected
                    } else {
                        bytes.len().saturating_sub(before)
                    };
                    bytes.extend(std::iter::repeat(0_u8).take(zeros));
                }
                if bytes.len() > MAX_QUEUED_BYTES {
                    // Disk stall: drop oldest audio to stay bounded rather
                    // than growing RAM without limit.
                    let overflow = bytes.len() - MAX_QUEUED_BYTES;
                    let drop_frames =
                        overflow.div_ceil(BYTES_PER_FRAME).max(1);
                    for _ in 0..drop_frames.saturating_mul(BYTES_PER_FRAME) {
                        bytes.pop_front();
                    }
                }
                // Keep draining while packets remain; the next
                // `get_next_packet_size` call decides when to stop.
            }
            while bytes.len() >= 4 {
                // Length was checked, so pops cannot fail; the `else break`
                // keeps this panic-free even under concurrent mutation.
                let (Some(b0), Some(b1), Some(b2), Some(b3)) = (
                    bytes.pop_front(),
                    bytes.pop_front(),
                    bytes.pop_front(),
                    bytes.pop_front(),
                ) else {
                    break;
                };
                let sample = f32::from_le_bytes([b0, b1, b2, b3]);
                // Defective drivers can emit NaN/Inf floats; store silence
                // instead of poisoning the WAV and downstream meters.
                let safe = if sample.is_finite() { sample } else { 0.0 };
                if let Err(e) = writer.write_sample(safe) {
                    return Err(AudioError::Device(format!(
                        "failed to write audio to '{}' ({e}); the disk may be full",
                        path.display()
                    )));
                }
                samples += 1;
            }
            // Timeout (not signalled) just means "no new audio yet" — loop
            // back and re-check `stop`. Disconnects surface as read errors
            // on the next drain pass.
            let _ = event.wait_for_event(100);
        }
        Ok(samples / 2)
    })();
    let _ = client.stop_stream();
    match writer.finalize() {
        Ok(()) => capture_result,
        Err(finalize_err) => {
            // Prefer the capture error (disconnect/disk-full) when both
            // failed; it is the actionable one for recovery UI.
            capture_result.err().unwrap_or_else(|| {
                AudioError::Device(format!(
                    "failed to finalize audio file '{}' ({finalize_err})",
                    path.display()
                ))
            });
            // Re-derive the error to return (avoids moving twice).
            match capture_result {
                Ok(_) => Err(AudioError::Device(format!(
                    "failed to finalize audio file '{}' ({finalize_err})",
                    path.display()
                ))),
                Err(e) => Err(e),
            }
        }
    }
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
