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

/// Minimum bytes for a video/audio segment to count as playable, ported from
/// Recordly's `MIN_VALID_RECORDED_VIDEO_BYTES` (1024). Headers alone
/// (e.g. a 44-byte WAV header or an MP4 skeleton) must not count as a
/// recovered source for the Phase 1 "separate playable sources" exit.
pub const MIN_VALID_MEDIA_SEGMENT_BYTES: u64 = 1024;

/// Upper bound for retained diagnostics events per sidecar. Without a cap a
/// 20-minute recording with per-second snapshots grows without bound and
/// slows every subsequent append (read-modify-write of the whole file).
pub const MAX_DIAGNOSTICS_EVENTS: usize = 200;

/// Recordly truncates retained process output to the last 12k chars so one
/// huge stderr dump cannot blow up the diagnostics sidecar.
pub const MAX_DIAGNOSTICS_TEXT_CHARS: usize = 12_000;

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
        // `x - left` can overflow `i32` for adversarial bounds (e.g.
        // `left = i32::MIN`, `x = i32::MAX`), which panics in debug builds.
        // Widen first so enumeration of corrupt window rects never panics.
        let nx = (i64::from(x) - i64::from(self.left)) as f64 / f64::from(self.width);
        let ny = (i64::from(y) - i64::from(self.top)) as f64 / f64::from(self.height);
        NormalizedPoint {
            x: nx.clamp(0.0, 1.0),
            y: ny.clamp(0.0, 1.0),
            inside: (0.0..=1.0).contains(&nx) && (0.0..=1.0).contains(&ny),
        }
    }

    #[must_use]
    pub fn physical_from_logical(&self, x: f64, y: f64, dpi: u32) -> (i32, i32) {
        // `GetDpiForWindow` can return 0 for a dead window; fall back to 96
        // so a stale DPI never collapses coordinates to the origin.
        let effective_dpi = if dpi == 0 { 96 } else { dpi };
        let scale = f64::from(effective_dpi) / 96.0;
        // NaN/negative/infinite logical coords and extreme DPI must not
        // panic via `as` overflow-then-`+` overflow. Clamp to a finite offset
        // and saturate the final addition.
        let dx = (x * scale).round();
        let dy = (y * scale).round();
        let dx = if dx.is_finite() {
            dx.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
        } else {
            0
        };
        let dy = if dy.is_finite() {
            dy.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
        } else {
            0
        };
        (self.left.saturating_add(dx), self.top.saturating_add(dy))
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
        // `duration_since` panics if `end < started`; `Instant` is monotonic
        // so this should be impossible, but a saturating path keeps a clock
        // glitch from panicking a 20-minute recording.
        end.saturating_duration_since(self.started)
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
        let payload = serde_json::to_vec_pretty(self)?;
        if let Err(e) = (|| -> Result<(), CaptureError> {
            fs::write(&temp, payload)?;
            OpenOptions::new().write(true).open(&temp)?.sync_all()?;
            fs::rename(&temp, &target)?;
            Ok(())
        })() {
            // A crashed commit must not leave a stale `.tmp` that a later
            // replay mistakes for progress; best-effort cleanup only.
            let _ = fs::remove_file(&temp);
            return Err(e);
        }
        Ok(())
    }
    pub fn replay(project_root: &Path) -> Result<Option<Self>, CaptureError> {
        let path = project_root.join("recovery/active-recording.json");
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(CaptureError::Io(e)),
        };
        let value: Self = serde_json::from_slice(&bytes)?;
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
                // A corrupt or hostile manifest must not probe arbitrary
                // paths: absolute entries and `..` escapes are skipped so
                // `project_root.join` cannot escape the project.
                if !is_safe_relative_segment_path(&segment.relative_path) {
                    return false;
                }
                let path = project_root.join(&segment.relative_path);
                let minimum = min_bytes_for_segment_kind(segment.kind);
                match fs::metadata(&path) {
                    Ok(meta) => meta.is_file() && meta.len() >= minimum,
                    Err(_) => false,
                }
            })
            .cloned()
            .collect()
    }
}

/// Rejects absolute paths, empty paths and any `..` escape so manifest
/// replay stays confined to the project root.
fn is_safe_relative_segment_path(path: &Path) -> bool {
    use std::path::Component;
    if path.as_os_str().is_empty() {
        return false;
    }
    if path.is_absolute() {
        return false;
    }
    let mut depth: i32 = 0;
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => return false,
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            Component::CurDir => {}
            Component::Normal(_) => depth += 1,
        }
    }
    depth > 0
}

/// Minimum file size for a segment to count as playable. Media tracks use
/// Recordly's 1024-byte floor; cursor/click telemetry is JSONL where even a
/// single line is meaningful recovery data.
#[must_use]
pub fn min_bytes_for_segment_kind(kind: SegmentKind) -> u64 {
    match kind {
        SegmentKind::Screen
        | SegmentKind::Microphone
        | SegmentKind::SystemAudio
        | SegmentKind::Camera => MIN_VALID_MEDIA_SEGMENT_BYTES,
        SegmentKind::Cursor | SegmentKind::Clicks => 1,
    }
}

/// Converts a stop-path `f64` duration in seconds to integer microseconds
/// without `as` truncation surprises: non-finite/negative durations map to 0
/// and huge durations saturate instead of wrapping.
#[must_use]
pub fn secs_f64_to_micros(duration_secs: f64) -> i64 {
    if !duration_secs.is_finite() || duration_secs <= 0.0 {
        return 0;
    }
    const MICROS_PER_SEC: f64 = 1_000_000.0;
    let micros = (duration_secs * MICROS_PER_SEC).round();
    if micros >= i64::MAX as f64 {
        i64::MAX
    } else {
        micros as i64
    }
}

/// Default H.264 bitrate for a capture size, ported from Recordly's
/// `calculateScreenRecordingBitrate` (18 Mbps base, 28 Mbps at QHD, 45 Mbps
/// at 4K, ×1.35 at 60 FPS) so a `bitrate: 0` config still yields a playable
/// file instead of a zero-byte output.
#[must_use]
pub fn default_video_bitrate(width: u32, height: u32, fps: u32) -> u32 {
    const FOUR_K_PIXELS: u64 = 3840 * 2160;
    const QHD_PIXELS: u64 = 2560 * 1440;
    const BITRATE_4K: f64 = 45_000_000.0;
    const BITRATE_QHD: f64 = 28_000_000.0;
    const BITRATE_BASE: f64 = 18_000_000.0;
    let pixels = u64::from(width.max(1)) * u64::from(height.max(1));
    let base = if pixels >= FOUR_K_PIXELS {
        BITRATE_4K
    } else if pixels >= QHD_PIXELS {
        BITRATE_QHD
    } else {
        BITRATE_BASE
    };
    let boost = if fps >= 60 { 1.35 } else { 1.0 };
    (base * boost + 0.5).clamp(1_000_000.0, 120_000_000.0) as u32
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
        match fs::read(&sidecar) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or(serde_json::json!({})),
            // A transient read failure (e.g. AV lock) must not fail the stop
            // path; start a fresh in-memory log for this append.
            Err(_) => serde_json::json!({}),
        }
    } else {
        serde_json::json!({})
    };
    // A corrupt sidecar (array, string, …) from a crashed writer resets to a
    // fresh log instead of failing every future append.
    if !log.is_object() {
        log = serde_json::json!({});
    }
    let Some(obj) = log.as_object_mut() else {
        return Err(CaptureError::InvalidConfig(
            "diagnostics sidecar is not an object".into(),
        ));
    };
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
    // A corrupt `events` (object/string from a torn write) resets to `[]`
    // rather than silently dropping this snapshot.
    if !events.is_array() {
        *events = serde_json::json!([]);
    }
    if let Some(list) = events.as_array_mut() {
        let mut event = serde_json::json!({ "timestamp": now });
        if let Some(map) = event.as_object_mut()
            && let Some(snap) = snapshot.as_object()
        {
            for (key, value) in snap {
                map.insert(key.clone(), truncate_diagnostics_value(value));
            }
        }
        list.push(event.clone());
        // Bound the file: keep the most recent events so a 20-minute session
        // with frequent snapshots stays small and appends stay fast.
        if list.len() > MAX_DIAGNOSTICS_EVENTS {
            let excess = list.len() - MAX_DIAGNOSTICS_EVENTS;
            list.drain(0..excess);
        }
        obj.insert("latest".into(), event);
    }
    fs::write(&sidecar, serde_json::to_vec_pretty(&log)?)?;
    Ok(sidecar)
}

/// Ports Recordly's `truncateDiagnosticsText`: over-long strings keep their
/// tail (the most recent output) plus a truncation marker.
fn truncate_diagnostics_value(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(text) => {
            serde_json::json!(truncate_diagnostics_text(text, MAX_DIAGNOSTICS_TEXT_CHARS))
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(truncate_diagnostics_value).collect())
        }
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), truncate_diagnostics_value(item)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn truncate_diagnostics_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let tail: String = text.chars().rev().take(max_chars).collect::<String>();
    let tail: String = tail.chars().rev().collect();
    format!("{tail}\n[truncated to last {max_chars} chars]")
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
    let (records, skipped) = read_jsonl_lenient(path)?;
    if skipped > 0 {
        return Err(CaptureError::InvalidConfig(format!(
            "telemetry file has {skipped} corrupt line(s): {}",
            path.display()
        )));
    }
    Ok(records)
}

/// Lossy JSONL replay for crash recovery: skips blank and corrupt lines and
/// reports how many were dropped instead of discarding the whole 20-minute
/// telemetry track because of one torn final line.
pub fn read_jsonl_lenient<T: for<'de> Deserialize<'de>>(
    path: &Path,
) -> Result<(Vec<TelemetryRecord<T>>, usize), CaptureError> {
    let file = File::open(path)?;
    let mut records = Vec::new();
    let mut skipped = 0_usize;
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(&line) {
            Ok(record) => records.push(record),
            Err(_) => skipped += 1,
        }
    }
    Ok((records, skipped))
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
        // Media segments need Recordly's 1024-byte floor to count as
        // playable; a 7-byte stub must not pass recovery.
        fs::write(temp.path().join("media/screen-0001.mp4"), vec![0xAB; 2048]).unwrap();
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
        // A 7-byte header-only stub is also unplayable for media tracks.
        fs::write(temp.path().join("media/screen-0002.mp4"), b"segment").unwrap();
        let mut manifest = RecoveryManifest::new(Uuid::new_v4(), ClockOrigin::now().unwrap());
        for path in ["media/screen-0001.mp4", "media/screen-0002.mp4"] {
            manifest.segments.push(RecoverySegment {
                id: Uuid::new_v4(),
                kind: SegmentKind::Screen,
                relative_path: path.into(),
                start_micros: 0,
                duration_micros: Some(1_000_000),
                finalized: true,
            });
        }
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

    // --- Heavy-load property tests: rect / DPI / clock math ---

    #[test]
    fn rect_normalize_property_never_panics_and_stays_clamped() {
        let bounds_cases = [
            RectI32 {
                left: 0,
                top: 0,
                width: 1920,
                height: 1080,
            },
            RectI32 {
                left: -1920,
                top: -300,
                width: 1920,
                height: 1080,
            },
            RectI32 {
                left: 0,
                top: 0,
                width: 0,
                height: 0,
            },
            RectI32 {
                left: 0,
                top: 0,
                width: -10,
                height: 600,
            },
            RectI32 {
                left: i32::MIN,
                top: i32::MIN,
                width: i32::MAX,
                height: i32::MAX,
            },
            RectI32 {
                left: i32::MAX,
                top: i32::MAX,
                width: 100,
                height: 100,
            },
        ];
        let points = [
            (0, 0),
            (960, 540),
            (-10_000, -10_000),
            (10_000, 10_000),
            (i32::MIN, i32::MIN),
            (i32::MAX, i32::MAX),
        ];
        for bounds in &bounds_cases {
            for (x, y) in &points {
                let p = bounds.normalize(*x, *y);
                assert!((0.0..=1.0).contains(&p.x), "x out of range: {p:?}");
                assert!((0.0..=1.0).contains(&p.y), "y out of range: {p:?}");
                if bounds.width <= 0 || bounds.height <= 0 {
                    assert!(!p.inside);
                    assert_eq!((p.x, p.y), (0.0, 0.0));
                }
            }
        }
    }

    #[test]
    fn physical_from_logical_property_saturates_without_panicking() {
        let bounds_cases = [
            RectI32 {
                left: 0,
                top: 0,
                width: 1920,
                height: 1080,
            },
            RectI32 {
                left: i32::MIN,
                top: i32::MIN,
                width: 100,
                height: 100,
            },
            RectI32 {
                left: i32::MAX - 10,
                top: i32::MAX - 10,
                width: 100,
                height: 100,
            },
        ];
        let coords = [
            0.0,
            100.5,
            -50.25,
            1920.0,
            1.0e9,
            -1.0e9,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ];
        let dpis = [0_u32, 1, 48, 96, 120, 144, 192, 288, u32::MAX];
        for bounds in &bounds_cases {
            for x in &coords {
                for y in &coords {
                    for dpi in &dpis {
                        // Must never panic, even for hostile inputs.
                        let (px, py) = bounds.physical_from_logical(*x, *y, *dpi);
                        // DPI 0 falls back to 96 so stale handles stay sane.
                        let (fallback_x, fallback_y) = bounds.physical_from_logical(*x, *y, 96);
                        if *dpi == 0 {
                            assert_eq!((px, py), (fallback_x, fallback_y));
                        }
                        // Ordinary case still exact.
                        let _ = (px, py);
                    }
                }
            }
        }
        // Regression anchors for the common DPI ladder.
        let bounds = RectI32 {
            left: 10,
            top: 20,
            width: 1920,
            height: 1080,
        };
        assert_eq!(bounds.physical_from_logical(100.0, 80.0, 96), (110, 100));
        assert_eq!(bounds.physical_from_logical(100.0, 80.0, 0), (110, 100));
        // NaN input saturates to the origin offset instead of panicking.
        assert_eq!(bounds.physical_from_logical(f64::NAN, 0.0, 96), (10, 20));
    }

    #[test]
    fn clock_ticks_property_covers_overflow_and_bad_frequency() {
        let origin = ClockOrigin {
            wall_time_utc: Utc::now(),
            qpc_ticks: 10_000,
            qpc_frequency: 10_000_000,
        };
        // Exact anchor: 10 ticks at 10 MHz == 1 microsecond.
        assert_eq!(origin.ticks_to_micros(10_010).unwrap(), 1);
        // Negative deltas stay negative (pause accounting relies on sign).
        assert_eq!(origin.ticks_to_micros(9_990).unwrap(), -1);
        // Far-future overflow errors instead of wrapping.
        assert!(origin.ticks_to_micros(i64::MAX).is_err());
        for bad_frequency in [0, -1, i64::MIN] {
            let bad = ClockOrigin {
                wall_time_utc: Utc::now(),
                qpc_ticks: 0,
                qpc_frequency: bad_frequency,
            };
            assert!(bad.ticks_to_micros(100).is_err());
        }
        // 20 minutes of ticks at 10 MHz converts exactly (sync budget needs
        // integer micros, not float seconds).
        let twenty_min_ticks = 10_000 + 10_000_000 * 60 * 20;
        assert_eq!(
            origin.ticks_to_micros(twenty_min_ticks).unwrap(),
            1_200_000_000
        );
    }

    #[test]
    fn secs_to_micros_conversion_is_saturating_and_monotonic() {
        assert_eq!(secs_f64_to_micros(f64::NAN), 0);
        assert_eq!(secs_f64_to_micros(f64::INFINITY), 0);
        assert_eq!(secs_f64_to_micros(-1.0), 0);
        assert_eq!(secs_f64_to_micros(0.0), 0);
        assert_eq!(secs_f64_to_micros(3.0), 3_000_000);
        assert_eq!(secs_f64_to_micros(1_200.0), 1_200_000_000);
        assert_eq!(secs_f64_to_micros(1e18), i64::MAX);
        // A chained 20-minute session stays gapless at segment boundaries.
        let mut cursor = 0_i64;
        for _ in 0..40 {
            let step = secs_f64_to_micros(30.0);
            cursor = cursor.saturating_add(step);
        }
        assert_eq!(cursor, 1_200_000_000);
    }

    #[test]
    fn default_bitrate_follows_recordly_tiers() {
        assert_eq!(default_video_bitrate(1920, 1080, 30), 18_000_000);
        assert_eq!(default_video_bitrate(1920, 1080, 60), 24_300_000);
        assert_eq!(default_video_bitrate(2560, 1440, 30), 28_000_000);
        assert_eq!(default_video_bitrate(3840, 2160, 30), 45_000_000);
        assert_eq!(default_video_bitrate(3840, 2160, 60), 60_750_000);
        // Degenerate dims still yield a playable bitrate, never 0.
        assert!(default_video_bitrate(0, 0, 30) > 0);
    }

    // --- Stress replay: corrupt manifests + multi-segment recovery ---

    #[test]
    fn replay_missing_manifest_returns_none_and_never_panics() {
        let temp = tempfile::tempdir().unwrap();
        assert!(RecoveryManifest::replay(temp.path()).unwrap().is_none());
        assert!(RecoveryManifest::replay_lenient(temp.path()).is_none());
    }

    #[test]
    fn replay_rejects_truncated_and_wrong_schema_manifests() {
        let temp = tempfile::tempdir().unwrap();
        // Torn write: half a JSON object (crash mid-commit before rename).
        fs::create_dir_all(temp.path().join("recovery")).unwrap();
        let manifest = RecoveryManifest::new(Uuid::new_v4(), ClockOrigin::now().unwrap());
        manifest.commit(temp.path()).unwrap();
        let full = fs::read(temp.path().join("recovery/active-recording.json")).unwrap();
        let half = &full[..full.len() / 2];
        fs::write(temp.path().join("recovery/active-recording.json"), half).unwrap();
        assert!(RecoveryManifest::replay(temp.path()).is_err());
        assert!(RecoveryManifest::replay_lenient(temp.path()).is_none());

        // Wrong schema version errors strictly (migration required).
        let mut value: serde_json::Value = serde_json::from_slice(&full).unwrap();
        value["schemaVersion"] = serde_json::json!(999);
        fs::write(
            temp.path().join("recovery/active-recording.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        assert!(RecoveryManifest::replay(temp.path()).is_err());
    }

    #[test]
    fn playable_segments_reject_absolute_and_escaping_paths() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("media")).unwrap();
        // A real file outside the project that a hostile manifest points at.
        let outside = temp.path().join("outside.mp4");
        fs::write(&outside, vec![0xAB; 4096]).unwrap();
        let mut manifest = RecoveryManifest::new(Uuid::new_v4(), ClockOrigin::now().unwrap());
        for (kind, path) in [
            (SegmentKind::Screen, outside.to_string_lossy().to_string()),
            (SegmentKind::Screen, String::from("../outside.mp4")),
            (SegmentKind::Screen, String::from("../../etc/passwd")),
            (SegmentKind::Screen, String::from("")),
            (SegmentKind::Screen, String::from("media/../../outside.mp4")),
        ] {
            manifest.segments.push(RecoverySegment {
                id: Uuid::new_v4(),
                kind,
                relative_path: path.into(),
                start_micros: 0,
                duration_micros: Some(1_000_000),
                finalized: true,
            });
        }
        manifest.commit(temp.path()).unwrap();
        let replayed = RecoveryManifest::replay(temp.path()).unwrap().unwrap();
        assert_eq!(replayed.playable_segments(temp.path()).len(), 0);
    }

    #[test]
    fn playable_segments_enforce_media_floor_but_keep_telemetry() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("media")).unwrap();
        fs::create_dir_all(temp.path().join("telemetry")).unwrap();
        // 1023 bytes: just under the media floor.
        fs::write(temp.path().join("media/screen-0001.mp4"), vec![0xAB; 1023]).unwrap();
        fs::write(temp.path().join("media/screen-0002.mp4"), vec![0xAB; 1024]).unwrap();
        // Telemetry JSONL: a single line is meaningful recovery data.
        fs::write(temp.path().join("telemetry/cursor.jsonl"), b"{}\n").unwrap();
        let mut manifest = RecoveryManifest::new(Uuid::new_v4(), ClockOrigin::now().unwrap());
        for (kind, path) in [
            (SegmentKind::Screen, "media/screen-0001.mp4"),
            (SegmentKind::Screen, "media/screen-0002.mp4"),
            (SegmentKind::Cursor, "telemetry/cursor.jsonl"),
        ] {
            manifest.segments.push(RecoverySegment {
                id: Uuid::new_v4(),
                kind,
                relative_path: path.into(),
                start_micros: 0,
                duration_micros: Some(1_000_000),
                finalized: true,
            });
        }
        manifest.commit(temp.path()).unwrap();
        let replayed = RecoveryManifest::replay(temp.path()).unwrap().unwrap();
        let playable = replayed.playable_segments(temp.path());
        assert_eq!(playable.len(), 2);
        assert!(
            playable
                .iter()
                .any(|s| s.relative_path == PathBuf::from("media/screen-0002.mp4"))
        );
        assert!(
            playable
                .iter()
                .any(|s| s.relative_path == PathBuf::from("telemetry/cursor.jsonl"))
        );
    }

    #[test]
    fn multi_segment_recovery_keeps_finalized_chain_across_crash() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("media")).unwrap();
        // Simulate a 4-segment 20-minute session: seg1+seg2 committed,
        // seg3 vanished (encode failed), seg4 still open at crash.
        for name in ["media/screen-0001.mp4", "media/screen-0002.mp4"] {
            fs::write(temp.path().join(name), vec![0xAB; 4096]).unwrap();
        }
        fs::write(temp.path().join("media/screen-0004.mp4"), vec![0xAB; 4096]).unwrap();
        let mut manifest = RecoveryManifest::new(Uuid::new_v4(), ClockOrigin::now().unwrap());
        let mut start = 0_i64;
        for (index, name) in [
            "media/screen-0001.mp4",
            "media/screen-0002.mp4",
            "media/screen-0003.mp4",
            "media/screen-0004.mp4",
        ]
        .iter()
        .enumerate()
        {
            let finalized = index < 2;
            manifest.segments.push(RecoverySegment {
                id: Uuid::new_v4(),
                kind: SegmentKind::Screen,
                relative_path: (*name).into(),
                start_micros: start,
                duration_micros: if finalized { Some(300_000_000) } else { None },
                finalized,
            });
            start += 300_000_000;
        }
        manifest.commit(temp.path()).unwrap();
        let replayed = RecoveryManifest::replay(temp.path()).unwrap().unwrap();
        let playable = replayed.playable_segments(temp.path());
        // Only the two committed segments with files on disk recover.
        assert_eq!(playable.len(), 2);
        assert_eq!(playable[0].start_micros, 0);
        assert_eq!(playable[1].start_micros, 300_000_000);
        // Chained starts stay gapless (sync budget depends on this).
        assert_eq!(
            playable[1].start_micros - playable[0].start_micros,
            300_000_000
        );
    }

    #[test]
    fn diagnostics_sidecar_caps_events_and_truncates_long_text() {
        let temp = tempfile::tempdir().unwrap();
        let video = temp.path().join("media/screen-0001.mp4");
        fs::create_dir_all(video.parent().unwrap()).unwrap();
        fs::write(&video, b"segment").unwrap();
        for index in 0..(MAX_DIAGNOSTICS_EVENTS + 50) {
            let snapshot =
                serde_json::json!({ "phase": "tick", "index": index, "pad": "x".repeat(100) });
            append_diagnostics_snapshot(&video, &snapshot).unwrap();
        }
        let log: serde_json::Value =
            serde_json::from_slice(&fs::read(diagnostics_sidecar_path(&video)).unwrap()).unwrap();
        assert_eq!(
            log["events"].as_array().unwrap().len(),
            MAX_DIAGNOSTICS_EVENTS
        );
        // The oldest events were evicted; the newest survived.
        assert_eq!(log["events"][0]["index"], serde_json::json!(50));
        assert_eq!(
            log["latest"]["index"],
            serde_json::json!(MAX_DIAGNOSTICS_EVENTS + 49)
        );

        // A 20k-char stderr dump truncates to the tail + marker.
        let huge = "y".repeat(MAX_DIAGNOSTICS_TEXT_CHARS + 5_000);
        append_diagnostics_snapshot(&video, &serde_json::json!({ "processOutput": huge })).unwrap();
        let log: serde_json::Value =
            serde_json::from_slice(&fs::read(diagnostics_sidecar_path(&video)).unwrap()).unwrap();
        let stored = log["latest"]["processOutput"].as_str().unwrap();
        assert!(stored.contains("truncated to last"));
        assert!(stored.chars().count() <= MAX_DIAGNOSTICS_TEXT_CHARS + 100);
    }

    #[test]
    fn diagnostics_sidecar_recovers_from_corrupt_existing_file() {
        let temp = tempfile::tempdir().unwrap();
        let video = temp.path().join("media/screen-0001.mp4");
        fs::create_dir_all(video.parent().unwrap()).unwrap();
        fs::write(&video, b"segment").unwrap();
        let sidecar = diagnostics_sidecar_path(&video);
        // Crashed writer left a bare array.
        fs::write(&sidecar, b"[1,2,3]").unwrap();
        append_diagnostics_snapshot(&video, &serde_json::json!({ "phase": "stop" })).unwrap();
        let log: serde_json::Value = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
        assert_eq!(log["events"].as_array().unwrap().len(), 1);

        // Torn write left a non-array `events`.
        let mut corrupt: serde_json::Value =
            serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
        corrupt["events"] = serde_json::json!("torn");
        fs::write(&sidecar, serde_json::to_vec(&corrupt).unwrap()).unwrap();
        append_diagnostics_snapshot(&video, &serde_json::json!({ "phase": "stop" })).unwrap();
        let log: serde_json::Value = serde_json::from_slice(&fs::read(&sidecar).unwrap()).unwrap();
        assert_eq!(log["events"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn jsonl_lenient_replay_skips_torn_final_line() {
        use serde::{Deserialize, Serialize};
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct Payload {
            value: i32,
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("telemetry/cursor.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        {
            let mut writer = JsonlWriter::append(&path).unwrap();
            for value in [1, 2, 3] {
                writer
                    .write(&TelemetryRecord {
                        schema_version: 1,
                        timestamp_micros: i64::from(value) * 1000,
                        payload: Payload { value },
                    })
                    .unwrap();
            }
        }
        // Clean round-trip first.
        let (records, skipped) = read_jsonl_lenient::<Payload>(&path).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(skipped, 0);
        // Crash mid-flush appends a torn final line plus a corrupt line.
        {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            writeln!(file, "{{not json}}").unwrap();
            writeln!(file).unwrap();
            write!(file, "{{\"torn\":").unwrap();
        }
        let (records, skipped) = read_jsonl_lenient::<Payload>(&path).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(skipped, 2);
        assert_eq!(records[0].payload.value, 1);
        // Strict replay still errors so callers that need exactness opt in.
        assert!(read_jsonl::<Payload>(&path).is_err());
    }
}
