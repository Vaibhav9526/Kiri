use chrono::{DateTime, Utc};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use thiserror::Error;
use uuid::Uuid;

pub const RECOVERY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum CaptureError {
    #[error("capture source is unavailable: {0}")]
    SourceUnavailable(String),
    #[error("capture is not supported: {0}")]
    Unsupported(String),
    #[error("invalid capture configuration: {0}")]
    InvalidConfig(String),
    #[error("capture I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("capture serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("native capture failed: {0}")]
    Native(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RectI32 {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedPoint {
    pub x: f64,
    pub y: f64,
    pub inside: bool,
}

impl RectI32 {
    #[must_use]
    pub fn normalize(&self, x: i32, y: i32) -> NormalizedPoint {
        if self.width <= 0 || self.height <= 0 {
            return NormalizedPoint {
                x: 0.0,
                y: 0.0,
                inside: false,
            };
        }
        let nx = f64::from(x - self.left) / f64::from(self.width);
        let ny = f64::from(y - self.top) / f64::from(self.height);
        NormalizedPoint {
            x: nx.clamp(0.0, 1.0),
            y: ny.clamp(0.0, 1.0),
            inside: (0.0..=1.0).contains(&nx) && (0.0..=1.0).contains(&ny),
        }
    }

    #[must_use]
    pub fn physical_from_logical(&self, x: f64, y: f64, dpi: u32) -> (i32, i32) {
        let scale = f64::from(dpi) / 96.0;
        (
            self.left + (x * scale).round() as i32,
            self.top + (y * scale).round() as i32,
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureSourceKind {
    Display,
    Window,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceAvailability {
    Available,
    Minimized,
    Closed,
    Protected,
    Invalid,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSource {
    pub id: String,
    pub kind: CaptureSourceKind,
    pub title: String,
    pub process_name: Option<String>,
    pub bounds: RectI32,
    pub dpi: u32,
    pub availability: SourceAvailability,
    pub thumbnail_data_url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClockOrigin {
    pub wall_time_utc: DateTime<Utc>,
    pub qpc_ticks: i64,
    pub qpc_frequency: i64,
}

impl ClockOrigin {
    pub fn now() -> Result<Self, CaptureError> {
        platform_clock_origin()
    }

    pub fn ticks_to_micros(&self, ticks: i64) -> Result<i64, CaptureError> {
        if self.qpc_frequency <= 0 {
            return Err(CaptureError::InvalidConfig(
                "QPC frequency must be positive".into(),
            ));
        }
        let delta = i128::from(ticks) - i128::from(self.qpc_ticks);
        let micros = delta
            .checked_mul(1_000_000)
            .ok_or_else(|| CaptureError::InvalidConfig("timestamp overflow".into()))?
            / i128::from(self.qpc_frequency);
        i64::try_from(micros).map_err(|_| CaptureError::InvalidConfig("timestamp overflow".into()))
    }
}

#[cfg(windows)]
fn platform_clock_origin() -> Result<ClockOrigin, CaptureError> {
    use ::windows::Win32::System::Performance::{
        QueryPerformanceCounter, QueryPerformanceFrequency,
    };
    let mut ticks = 0_i64;
    let mut frequency = 0_i64;
    unsafe {
        QueryPerformanceCounter(&mut ticks).map_err(|e| CaptureError::Native(e.to_string()))?;
        QueryPerformanceFrequency(&mut frequency)
            .map_err(|e| CaptureError::Native(e.to_string()))?;
    }
    Ok(ClockOrigin {
        wall_time_utc: Utc::now(),
        qpc_ticks: ticks,
        qpc_frequency: frequency,
    })
}

#[cfg(not(windows))]
fn platform_clock_origin() -> Result<ClockOrigin, CaptureError> {
    Ok(ClockOrigin {
        wall_time_utc: Utc::now(),
        qpc_ticks: 0,
        qpc_frequency: 1_000_000_000,
    })
}

#[derive(Clone, Debug)]
pub struct RecordingClock {
    origin: ClockOrigin,
    started: Instant,
    paused_at: Option<Instant>,
    paused_total: Duration,
}

impl RecordingClock {
    pub fn start() -> Result<Self, CaptureError> {
        Ok(Self {
            origin: ClockOrigin::now()?,
            started: Instant::now(),
            paused_at: None,
            paused_total: Duration::ZERO,
        })
    }
    #[must_use]
    pub fn origin(&self) -> &ClockOrigin {
        &self.origin
    }
    #[must_use]
    pub fn elapsed_micros(&self) -> i64 {
        let end = self.paused_at.unwrap_or_else(Instant::now);
        end.duration_since(self.started)
            .saturating_sub(self.paused_total)
            .as_micros()
            .min(i64::MAX as u128) as i64
    }
    pub fn pause(&mut self) {
        self.paused_at.get_or_insert_with(Instant::now);
    }
    pub fn resume(&mut self) {
        if let Some(paused_at) = self.paused_at.take() {
            self.paused_total += paused_at.elapsed();
        }
    }
    /// Total time spent paused, excluding any currently-open pause interval
    /// plus the open interval when paused. Mirrors Recordly's
    /// pause-segment accounting so session manifests can report
    /// `pausedDuration` accurately.
    #[must_use]
    pub fn paused_duration_micros(&self) -> i64 {
        let mut total = self.paused_total;
        if let Some(paused_at) = self.paused_at {
            total += paused_at.elapsed();
        }
        total.as_micros().min(i64::MAX as u128) as i64
    }
}

pub struct BoundedSender<T> {
    sender: Sender<T>,
    dropped: Arc<AtomicU64>,
}
pub struct BoundedReceiver<T> {
    receiver: Receiver<T>,
    dropped: Arc<AtomicU64>,
}

pub fn bounded_queue<T>(
    capacity: usize,
) -> Result<(BoundedSender<T>, BoundedReceiver<T>), CaptureError> {
    if capacity == 0 {
        return Err(CaptureError::InvalidConfig(
            "queue capacity must be positive".into(),
        ));
    }
    let (sender, receiver) = bounded(capacity);
    let dropped = Arc::new(AtomicU64::new(0));
    Ok((
        BoundedSender {
            sender,
            dropped: Arc::clone(&dropped),
        },
        BoundedReceiver { receiver, dropped },
    ))
}

impl<T> BoundedSender<T> {
    pub fn try_send(&self, value: T) -> Result<(), T> {
        match self.sender.try_send(value) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(value) | TrySendError::Disconnected(value)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                Err(value)
            }
        }
    }
}

impl<T> BoundedReceiver<T> {
    pub fn recv_timeout(&self, timeout: Duration) -> Option<T> {
        self.receiver.recv_timeout(timeout).ok()
    }
    #[must_use]
    pub fn depth(&self) -> usize {
        self.receiver.len()
    }
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SegmentKind {
    Screen,
    Microphone,
    SystemAudio,
    Camera,
    Cursor,
    Clicks,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverySegment {
    pub id: Uuid,
    pub kind: SegmentKind,
    pub relative_path: PathBuf,
    pub start_micros: i64,
    pub duration_micros: Option<i64>,
    pub finalized: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryManifest {
    pub schema_version: u32,
    pub session_id: Uuid,
    pub project_id: Uuid,
    pub clock: ClockOrigin,
    pub active: bool,
    pub segments: Vec<RecoverySegment>,
    pub events: Vec<String>,
}

impl RecoveryManifest {
    pub fn new(project_id: Uuid, clock: ClockOrigin) -> Self {
        Self {
            schema_version: RECOVERY_SCHEMA_VERSION,
            session_id: Uuid::new_v4(),
            project_id,
            clock,
            active: true,
            segments: Vec::new(),
            events: Vec::new(),
        }
    }
    pub fn commit(&self, project_root: &Path) -> Result<(), CaptureError> {
        let recovery = project_root.join("recovery");
        fs::create_dir_all(&recovery)?;
        let target = recovery.join("active-recording.json");
        let temp = recovery.join("active-recording.json.tmp");
        fs::write(&temp, serde_json::to_vec_pretty(self)?)?;
        OpenOptions::new().write(true).open(&temp)?.sync_all()?;
        fs::rename(temp, target)?;
        Ok(())
    }
    pub fn replay(project_root: &Path) -> Result<Option<Self>, CaptureError> {
        let path = project_root.join("recovery/active-recording.json");
        if !path.exists() {
            return Ok(None);
        }
        let value: Self = serde_json::from_slice(&fs::read(path)?)?;
        if value.schema_version != RECOVERY_SCHEMA_VERSION {
            return Err(CaptureError::InvalidConfig(
                "unsupported recovery manifest".into(),
            ));
        }
        Ok(Some(value))
    }
    /// Lenient replay for listing candidates. A single corrupt manifest must
    /// not fail the whole recent-projects scan (Recordly skips unreadable
    /// diagnostics the same way). Strict [`Self::replay`] is still used when
    /// the user explicitly chooses to recover a project.
    pub fn replay_lenient(project_root: &Path) -> Option<Self> {
        Self::replay(project_root).ok()?
    }
    pub fn playable_segments(&self, project_root: &Path) -> Vec<RecoverySegment> {
        self.segments
            .iter()
            .filter(|segment| {
                if !segment.finalized {
                    return false;
                }
                let path = project_root.join(&segment.relative_path);
                // Recordly rejects empty/tiny outputs; Kiri keeps the cheaper
                // `> 0 bytes` check so existing fixtures (7-byte stubs) stay
                // playable while zero-byte partials are skipped.
                match fs::metadata(&path) {
                    Ok(meta) => meta.is_file() && meta.len() > 0,
                    Err(_) => false,
                }
            })
            .cloned()
            .collect()
    }
}

/// Recordly companion conventions, kept additive so the existing `.kiri`
/// layout (`media/screen-0001.mp4`, `media/microphone-0001.wav`, …) never
/// changes. New sidecars reuse Recordly's suffixes:
///
/// - `<video-stem>.recording-diagnostics.json` (diagnostics log)
/// - `<audio-path>.json` with `{ "startDelayMs": n }` (timing metadata)
#[must_use]
pub fn diagnostics_sidecar_path(video_path: &Path) -> PathBuf {
    // `with_extension` replaces only the final extension, so
    // `screen-0001.mp4` becomes `screen-0001.recording-diagnostics.json`,
    // matching Recordly's `getRecordingDiagnosticsPath`.
    video_path.with_extension("recording-diagnostics.json")
}

/// Appends `.json` (does not replace the audio extension), matching
/// Recordly's `${sidecarPath}.json` timing sidecars.
#[must_use]
pub fn audio_timing_sidecar_path(audio_path: &Path) -> PathBuf {
    let mut text = audio_path.as_os_str().to_owned();
    text.push(".json");
    PathBuf::from(text)
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioTimingMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_delay_ms: Option<i64>,
}

impl AudioTimingMetadata {
    /// Best-effort write of the timing sidecar; failures are returned so the
    /// caller can record them in diagnostics instead of failing the stop path.
    pub fn write_sidecar(
        audio_path: &Path,
        start_delay_ms: Option<i64>,
    ) -> Result<(), CaptureError> {
        let sidecar = audio_timing_sidecar_path(audio_path);
        if let Some(parent) = sidecar.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            &sidecar,
            serde_json::to_vec_pretty(&Self { start_delay_ms })?,
        )?;
        Ok(())
    }
}

/// Append-only diagnostics log written next to each screen segment, mirroring
/// Recordly's `<video>.recording-diagnostics.json` event log shape
/// (`{ version, createdAt, updatedAt, videoPath, events, latest }`).
pub fn append_diagnostics_snapshot(
    video_path: &Path,
    snapshot: &serde_json::Value,
) -> Result<PathBuf, CaptureError> {
    let sidecar = diagnostics_sidecar_path(video_path);
    if let Some(parent) = sidecar.parent() {
        fs::create_dir_all(parent)?;
    }
    let now = Utc::now().to_rfc3339();
    let mut log: serde_json::Value = if sidecar.is_file() {
        serde_json::from_slice(&fs::read(&sidecar)?).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    let obj = log.as_object_mut().ok_or_else(|| {
        CaptureError::InvalidConfig("diagnostics sidecar is not an object".into())
    })?;
    if obj.get("version").is_none() {
        obj.insert("version".into(), serde_json::json!(1));
    }
    if obj.get("createdAt").is_none() {
        obj.insert("createdAt".into(), serde_json::json!(now.clone()));
    }
    obj.insert("updatedAt".into(), serde_json::json!(now.clone()));
    obj.insert(
        "videoPath".into(),
        serde_json::json!(video_path.to_string_lossy().replace('\\', "/")),
    );
    obj.insert(
        "diagnosticsPath".into(),
        serde_json::json!(sidecar.to_string_lossy().replace('\\', "/")),
    );
    let events = obj
        .entry("events".to_string())
        .or_insert_with(|| serde_json::json!([]));
    if let Some(list) = events.as_array_mut() {
        let mut event = serde_json::json!({ "timestamp": now });
        if let Some(map) = event.as_object_mut()
            && let Some(snap) = snapshot.as_object()
        {
            for (key, value) in snap {
                map.insert(key.clone(), value.clone());
            }
        }
        list.push(event.clone());
        obj.insert("latest".into(), event);
    }
    fs::write(&sidecar, serde_json::to_vec_pretty(&log)?)?;
    Ok(sidecar)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureDiagnostics {
    pub source_fps: f64,
    pub encoded_frames: u64,
    pub dropped_frames: u64,
    pub queue_depth: usize,
    pub encoder: String,
    pub gpu_path: bool,
    pub audio_drift_millis: f64,
    pub device_events: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);
impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryRecord<T> {
    pub schema_version: u32,
    pub timestamp_micros: i64,
    pub payload: T,
}

pub struct JsonlWriter {
    writer: BufWriter<File>,
}
impl JsonlWriter {
    pub fn append(path: &Path) -> Result<Self, CaptureError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(Self {
            writer: BufWriter::new(OpenOptions::new().create(true).append(true).open(path)?),
        })
    }
    pub fn write<T: Serialize>(&mut self, record: &TelemetryRecord<T>) -> Result<(), CaptureError> {
        serde_json::to_writer(&mut self.writer, record)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }
}

pub fn read_jsonl<T: for<'de> Deserialize<'de>>(
    path: &Path,
) -> Result<Vec<TelemetryRecord<T>>, CaptureError> {
    BufReader::new(File::open(path)?)
        .lines()
        .map(|line| Ok(serde_json::from_str(&line?)?))
        .collect()
}

#[cfg(windows)]
pub mod windows;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_conversion_is_integer_and_relative() {
        let origin = ClockOrigin {
            wall_time_utc: Utc::now(),
            qpc_ticks: 10_000,
            qpc_frequency: 10_000_000,
        };
        assert_eq!(origin.ticks_to_micros(10_010).unwrap(), 1);
        assert_eq!(origin.ticks_to_micros(20_010_000).unwrap(), 2_000_000);
    }

    #[test]
    fn normalized_coordinates_clamp_and_report_inside() {
        let bounds = RectI32 {
            left: 100,
            top: 50,
            width: 800,
            height: 600,
        };
        assert_eq!(
            bounds.normalize(500, 350),
            NormalizedPoint {
                x: 0.5,
                y: 0.5,
                inside: true
            }
        );
        assert_eq!(
            bounds.normalize(50, 800),
            NormalizedPoint {
                x: 0.0,
                y: 1.0,
                inside: false
            }
        );
    }

    #[test]
    fn dpi_mapping_uses_physical_pixels() {
        let bounds = RectI32 {
            left: 10,
            top: 20,
            width: 1920,
            height: 1080,
        };
        assert_eq!(bounds.physical_from_logical(100.0, 80.0, 120), (135, 120));
        assert_eq!(bounds.physical_from_logical(100.0, 80.0, 144), (160, 140));
    }

    #[test]
    fn bounded_queue_drops_without_blocking_and_cancels() {
        let (sender, receiver) = bounded_queue(1).unwrap();
        assert!(sender.try_send(1).is_ok());
        assert_eq!(sender.try_send(2), Err(2));
        assert_eq!(receiver.dropped(), 1);
        assert_eq!(receiver.recv_timeout(Duration::from_millis(1)), Some(1));
        let token = CancellationToken::default();
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn recovery_replay_keeps_only_finalized_existing_segments() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("media")).unwrap();
        fs::write(temp.path().join("media/screen-0001.mp4"), b"segment").unwrap();
        let mut manifest = RecoveryManifest::new(Uuid::new_v4(), ClockOrigin::now().unwrap());
        manifest.segments.push(RecoverySegment {
            id: Uuid::new_v4(),
            kind: SegmentKind::Screen,
            relative_path: "media/screen-0001.mp4".into(),
            start_micros: 0,
            duration_micros: Some(1_000_000),
            finalized: true,
        });
        manifest.segments.push(RecoverySegment {
            id: Uuid::new_v4(),
            kind: SegmentKind::Camera,
            relative_path: "media/camera-0001.mp4".into(),
            start_micros: 0,
            duration_micros: None,
            finalized: false,
        });
        manifest.commit(temp.path()).unwrap();
        let replayed = RecoveryManifest::replay(temp.path()).unwrap().unwrap();
        assert_eq!(replayed.playable_segments(temp.path()).len(), 1);
    }

    #[test]
    fn empty_files_are_not_playable_and_corrupt_manifest_is_skipped_leniently() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("media")).unwrap();
        fs::write(temp.path().join("media/screen-0001.mp4"), b"").unwrap();
        let mut manifest = RecoveryManifest::new(Uuid::new_v4(), ClockOrigin::now().unwrap());
        manifest.segments.push(RecoverySegment {
            id: Uuid::new_v4(),
            kind: SegmentKind::Screen,
            relative_path: "media/screen-0001.mp4".into(),
            start_micros: 0,
            duration_micros: Some(1_000_000),
            finalized: true,
        });
        manifest.commit(temp.path()).unwrap();
        let replayed = RecoveryManifest::replay(temp.path()).unwrap().unwrap();
        assert_eq!(replayed.playable_segments(temp.path()).len(), 0);
        // Corrupt the manifest: strict replay errors, lenient replay skips.
        fs::write(
            temp.path().join("recovery/active-recording.json"),
            b"{not json",
        )
        .unwrap();
        assert!(RecoveryManifest::replay(temp.path()).is_err());
        assert!(RecoveryManifest::replay_lenient(temp.path()).is_none());
    }

    #[test]
    fn recordly_sidecar_naming_matches_reference_conventions() {
        let video = Path::new("media/screen-0001.mp4");
        assert_eq!(
            diagnostics_sidecar_path(video),
            PathBuf::from("media/screen-0001.recording-diagnostics.json")
        );
        let audio = Path::new("media/microphone-0001.wav");
        assert_eq!(
            audio_timing_sidecar_path(audio),
            PathBuf::from("media/microphone-0001.wav.json")
        );
    }

    #[test]
    fn diagnostics_sidecar_appends_events_like_recordly() {
        let temp = tempfile::tempdir().unwrap();
        let video = temp.path().join("media/screen-0001.mp4");
        fs::create_dir_all(video.parent().unwrap()).unwrap();
        fs::write(&video, b"segment").unwrap();
        let snapshot = serde_json::json!({ "phase": "stop", "encodedFrames": 10 });
        let sidecar = append_diagnostics_snapshot(&video, &snapshot).unwrap();
        assert_eq!(sidecar, diagnostics_sidecar_path(&video));
        let log: serde_json::Value = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
        assert_eq!(log["version"], serde_json::json!(1));
        assert_eq!(log["events"].as_array().unwrap().len(), 1);
        append_diagnostics_snapshot(&video, &snapshot).unwrap();
        let log: serde_json::Value = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
        assert_eq!(log["events"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn paused_duration_tracks_open_pause_interval() {
        let mut clock = RecordingClock::start().unwrap();
        assert_eq!(clock.paused_duration_micros(), 0);
        clock.pause();
        std::thread::sleep(Duration::from_millis(5));
        assert!(clock.paused_duration_micros() >= 1_000);
        clock.resume();
        assert!(clock.paused_duration_micros() >= 1_000);
    }
}
