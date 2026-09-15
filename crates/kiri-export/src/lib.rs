//! Deterministic local MP4 export planning + execution scaffolding.
//!
//! Recordly mapping: `src/lib/exporter/` (WebCodecs + mediabunny in the
//! browser, native compositor fast paths) becomes a Rust-owned pipeline:
//! the preview renderer and this exporter share the same scene math so that
//! preview and exported frames match for the golden scenes (Phase 2 gate).
//!
//! This crate owns presets, request validation, deterministic ffmpeg
//! argument planning, headless scene math (the preview==export contract),
//! PCM-peak silence detection (Phase 3, rule-based only), and a blocking
//! ffmpeg execution runner with progress parsing and cooperative
//! cancellation. Frame rendering follows in a later slice. AI features
//! (captions burn-in from transcription, auto-zoom models) stay out until
//! their explicit phase: [`suggest_zoom_regions`] lives in `kiri-project`
//! and is purely heuristic.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("invalid export plan: {0}")]
    Invalid(String),
    #[error("export cancelled")]
    Cancelled,
    #[error("ffmpeg exited with status {status}: {tail}")]
    FfmpegFailed { status: String, tail: String },
    #[error("export I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Frame rates accepted for MP4 export.
/// Mirrors Recordly `MP4_FRAME_RATES = [24, 30, 60]`.
pub const VALID_MP4_FRAME_RATES: &[u32] = &[24, 30, 60];

/// Floor for every MP4 delivery bitrate, in bits per second.
/// Mirrors Recordly `MIN_MP4_BITRATE = 2_000_000` (`exportBitrate.ts`):
/// no fps/mode/quality combination may plan below this.
pub const MIN_MP4_BITRATE_BPS: u32 = 2_000_000;

/// Reference frame rate for bitrate scaling (`exportBitrate.ts`
/// `REFERENCE_FRAME_RATE`). 24 fps and 30 fps share the same multiplier;
/// only higher rates scale up via `sqrt(fps / 30)`.
pub const REFERENCE_FRAME_RATE_FPS: f64 = 30.0;

/// Maximum number of auxiliary audio inputs (`-i` flags besides the video).
/// Mirrors the pre-existing validation limit; now named so plans, errors,
/// and tests share one source of truth.
pub const MAX_AUDIO_INPUTS: usize = 8;

/// How many trailing stderr lines are kept for `FfmpegFailed` diagnostics.
pub const FFMPEG_TAIL_LINES: usize = 20;

#[must_use]
pub fn is_valid_mp4_frame_rate(fps: u32) -> bool {
    VALID_MP4_FRAME_RATES.contains(&fps)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportPreset {
    #[default]
    #[serde(rename = "1080p")]
    FullHd,
    #[serde(rename = "720p")]
    Hd,
}

impl ExportPreset {
    #[must_use]
    pub fn dimensions(self) -> (u32, u32) {
        match self {
            Self::FullHd => (1920, 1080),
            Self::Hd => (1280, 720),
        }
    }

    #[must_use]
    pub fn video_bitrate(self) -> &'static str {
        match self {
            Self::FullHd => "12M",
            Self::Hd => "8M",
        }
    }

    #[must_use]
    pub fn base_bitrate_bps(self) -> u32 {
        match self {
            Self::FullHd => 12_000_000,
            Self::Hd => 8_000_000,
        }
    }
}

/// Delivery bitrate policy. Mirrors Recordly `exportBitrate.ts`:
/// base bitrate scaled by frame-rate (`sqrt(max(1, fps/30))`) and by
/// encoding mode (fast 0.5, balanced 0.8, quality 1.0), floored at 2 Mbps.
///
/// A single delivery policy covers every backend: Recordly removed the
/// second set of native static-layout floors/caps that could more than
/// double the requested web-delivery target, so this crate does the same —
/// [`mp4_export_bitrate_bps`] applies only [`MIN_MP4_BITRATE_BPS`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EncodingMode {
    Fast,
    #[default]
    Balanced,
    Quality,
}

impl EncodingMode {
    #[must_use]
    pub fn ffmpeg_preset(self) -> &'static str {
        match self {
            Self::Fast => "veryfast",
            Self::Balanced => "medium",
            Self::Quality => "slow",
        }
    }

    #[must_use]
    pub fn crf(self) -> u8 {
        match self {
            Self::Fast => 20,
            Self::Balanced => 18,
            Self::Quality => 16,
        }
    }

    #[must_use]
    pub fn bitrate_multiplier(self) -> f64 {
        match self {
            Self::Fast => 0.5,
            Self::Balanced => 0.8,
            Self::Quality => 1.0,
        }
    }

    /// Keyframe interval in seconds per mode. Ports Recordly
    /// `exportTuning.ts` `KEYFRAME_INTERVAL_SECONDS` (fast 4 s, balanced
    /// 3 s, quality 2.5 s). Exposed so a future ffmpeg planner can emit a
    /// deterministic `-g` without re-deriving constants.
    #[must_use]
    pub fn keyframe_interval_seconds(self) -> f64 {
        match self {
            Self::Fast => 4.0,
            Self::Balanced => 3.0,
            Self::Quality => 2.5,
        }
    }
}

/// Export quality selector. Mirrors Recordly `ExportQuality`
/// (`"medium" | "good" | "high" | "source"`).
///
/// `Source` preserves the recorded pixel budget (8 Mbps at/below 720p,
/// 12 Mbps at/below 1080p, up to 45 Mbps at 4K with linear interpolation
/// between 1080p and 4K). The other levels share the web-delivery budget
/// (5/8/35 Mbps). Defaults to `Source` so preset-only requests keep their
/// historic bitrates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportQuality {
    Medium,
    Good,
    High,
    #[default]
    Source,
}

const HD_PIXELS: u64 = 1280 * 720;
const FULL_HD_PIXELS: u64 = 1920 * 1080;
const UHD_PIXELS: u64 = 3840 * 2160;

/// Frame-rate multiplier shared by every bitrate helper.
/// Ports `getFrameRateBitrateMultiplier`: 24 fps and 30 fps share the same
/// multiplier (1.0); higher rates scale with `sqrt(fps / 30)`.
#[must_use]
pub fn frame_rate_bitrate_multiplier(fps: u32) -> f64 {
    (f64::from(fps.max(1)) / REFERENCE_FRAME_RATE_FPS)
        .max(1.0)
        .sqrt()
}

fn interpolate_bitrate(
    total_pixels: u64,
    start_pixels: u64,
    end_pixels: u64,
    start_bitrate: u32,
    end_bitrate: u32,
) -> u32 {
    if end_pixels <= start_pixels {
        return start_bitrate;
    }
    let progress = (total_pixels.saturating_sub(start_pixels) as f64
        / (end_pixels - start_pixels) as f64)
        .clamp(0.0, 1.0);
    (f64::from(start_bitrate) + f64::from(end_bitrate - start_bitrate) * progress).round() as u32
}

/// Source-quality bitrate ceiling for an output size. Ports
/// `getSourceQualityBitrate`: 8 Mbps at/below 720p, 12 Mbps at/below 1080p,
/// 45 Mbps at/above 4K, linearly interpolated between 1080p and 4K.
///
/// Total over all inputs: zero sizes fall in the smallest bucket.
#[must_use]
pub fn source_quality_bitrate_bps(width: u32, height: u32) -> u32 {
    let total = u64::from(width) * u64::from(height);
    if total <= HD_PIXELS {
        8_000_000
    } else if total <= FULL_HD_PIXELS {
        12_000_000
    } else if total >= UHD_PIXELS {
        45_000_000
    } else {
        interpolate_bitrate(total, FULL_HD_PIXELS, UHD_PIXELS, 12_000_000, 45_000_000)
    }
}

/// Base (30 fps, quality-mode) bitrate for an output size. Ports
/// `getBaseMp4ExportBitrate`: `Source` uses [`source_quality_bitrate_bps`];
/// every other quality shares the web-delivery budget (5/8/35 Mbps with the
/// same 1080p→4K interpolation).
#[must_use]
pub fn base_mp4_bitrate_bps(width: u32, height: u32, quality: ExportQuality) -> u32 {
    if quality == ExportQuality::Source {
        return source_quality_bitrate_bps(width, height);
    }
    let total = u64::from(width) * u64::from(height);
    if total <= HD_PIXELS {
        5_000_000
    } else if total <= FULL_HD_PIXELS {
        8_000_000
    } else if total >= UHD_PIXELS {
        35_000_000
    } else {
        interpolate_bitrate(total, FULL_HD_PIXELS, UHD_PIXELS, 8_000_000, 35_000_000)
    }
}

/// Delivery bitrate for an explicit output size. Ports `getMp4ExportBitrate`:
/// `round(base * fps_multiplier * mode_multiplier)`, floored at
/// [`MIN_MP4_BITRATE_BPS`]. Pure so golden tests can snapshot policy
/// byte-for-byte.
#[must_use]
pub fn mp4_export_bitrate_bps(
    width: u32,
    height: u32,
    fps: u32,
    quality: ExportQuality,
    mode: EncodingMode,
) -> u32 {
    let requested = (f64::from(base_mp4_bitrate_bps(width, height, quality))
        * frame_rate_bitrate_multiplier(fps)
        * mode.bitrate_multiplier())
    .round() as u32;
    requested.max(MIN_MP4_BITRATE_BPS)
}

/// Target delivery bitrate in bits per second for a preset/fps/mode triple.
/// Preset dimensions are priced at [`ExportQuality::Source`] so this stays
/// byte-identical to [`mp4_export_bitrate_bps`] for the preset sizes.
/// Pure function so golden tests can snapshot export policy byte-for-byte.
#[must_use]
pub fn target_bitrate_bps(preset: ExportPreset, fps: u32, mode: EncodingMode) -> u32 {
    let (width, height) = preset.dimensions();
    mp4_export_bitrate_bps(width, height, fps, ExportQuality::Source, mode)
}

/// Deterministic keyframe (`-g`) interval in frames for an fps/mode pair.
/// Ports `getWebCodecsKeyFrameInterval` (`round(fps * seconds)`, at least 1)
/// so ffmpeg and WebCodecs plans share one interval policy.
#[must_use]
pub fn keyframe_interval_frames(fps: u32, mode: EncodingMode) -> u32 {
    ((f64::from(fps.max(1)) * mode.keyframe_interval_seconds()).round() as u32).max(1)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub project_id: Uuid,
    pub preset: ExportPreset,
    pub fps: u32,
    #[serde(default)]
    pub encoding_mode: EncodingMode,
    pub video_input: PathBuf,
    #[serde(default)]
    pub audio_inputs: Vec<PathBuf>,
    pub output: PathBuf,
}

impl ExportRequest {
    pub fn validate(&self) -> Result<(), ExportError> {
        if !is_valid_mp4_frame_rate(self.fps) {
            return Err(ExportError::Invalid(format!(
                "unsupported export fps {}; expected one of 24, 30, 60",
                self.fps
            )));
        }
        validate_media_path(&self.video_input, "video input is required")?;
        validate_media_path(&self.output, "export output is required")?;
        let is_mp4 = self
            .output
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"));
        if !is_mp4 {
            return Err(ExportError::Invalid(
                "export output must use an .mp4 extension".into(),
            ));
        }
        if self.audio_inputs.len() > MAX_AUDIO_INPUTS {
            return Err(ExportError::Invalid(format!(
                "at most {MAX_AUDIO_INPUTS} audio inputs are supported"
            )));
        }
        for (index, audio) in self.audio_inputs.iter().enumerate() {
            validate_media_path(audio, &format!("audio input #{index} is invalid"))?;
        }
        // Overwriting an input destroys the source (`-y` is always passed),
        // so output collisions are rejected before any process spawns.
        // Comparison is slash- and case-normalized so `a\b` matches `a/b`
        // on Windows checkouts.
        let output_key = normalized_path_key(&self.output);
        if normalized_path_key(&self.video_input) == output_key {
            return Err(ExportError::Invalid(
                "export output must differ from the video input".into(),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for audio in &self.audio_inputs {
            let key = normalized_path_key(audio);
            if key == output_key {
                return Err(ExportError::Invalid(
                    "export output must differ from every audio input".into(),
                ));
            }
            if !seen.insert(key) {
                return Err(ExportError::Invalid(
                    "duplicate audio inputs are not allowed".into(),
                ));
            }
        }
        Ok(())
    }
}

/// Reject empty/whitespace paths, embedded NULs (which would fail inside
/// `Command`), and flag-like paths (which ffmpeg would parse as options).
fn validate_media_path(path: &Path, empty_message: &str) -> Result<(), ExportError> {
    let text = path.to_string_lossy();
    if text.trim().is_empty() {
        return Err(ExportError::Invalid(empty_message.into()));
    }
    if text.contains('\0') {
        return Err(ExportError::Invalid("media path contains NUL".into()));
    }
    if is_flag_like_path(&text) {
        return Err(ExportError::Invalid(
            "media path must not look like a command-line flag".into(),
        ));
    }
    Ok(())
}

fn is_flag_like_path(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with('-') && trimmed.len() > 1
}

fn normalized_path_key(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPlan {
    pub preset: ExportPreset,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    #[serde(default)]
    pub encoding_mode: EncodingMode,
    /// Advisory delivery bitrate from [`target_bitrate_bps`] (floored at
    /// [`MIN_MP4_BITRATE_BPS`]). The current ffmpeg template stays
    /// CRF-based, so this is metadata for progress/observability and for a
    /// future VBV clamp — it never changes encode semantics today.
    #[serde(default)]
    pub bitrate_bps: u32,
    pub output: PathBuf,
    pub ffmpeg_args: Vec<String>,
}

/// Builds a deterministic ffmpeg argument list: H.264 video + AAC audio in
/// MP4, `+faststart` for progressive playback. Argument order is fixed so
/// golden tests can snapshot the plan byte-for-byte.
///
/// `encoding_mode` selects the x264 preset/CRF pair:
/// fast = veryfast/crf20, balanced = medium/crf18, quality = slow/crf16.
///
/// Audio mapping mirrors the inputs one-for-one: with no auxiliary audio the
/// plan maps video only; with N audio inputs it maps `1:a:0?` … `N:a:0?`.
/// (Previously the plan always emitted a dangling `1:a:0?` and silently
/// dropped every audio input past the first.)
pub fn plan_export(request: &ExportRequest) -> Result<ExportPlan, ExportError> {
    request.validate()?;
    let (width, height) = request.preset.dimensions();
    let bitrate_bps = target_bitrate_bps(request.preset, request.fps, request.encoding_mode);
    let mut args = vec!["-y".into(), "-i".into(), path_arg(&request.video_input)];
    for audio in &request.audio_inputs {
        args.push("-i".into());
        args.push(path_arg(audio));
    }
    args.extend(["-map", "0:v:0"].iter().map(ToString::to_string));
    for index in 0..request.audio_inputs.len() {
        args.push("-map".into());
        args.push(format!("{}:a:0?", index + 1));
    }
    args.extend(
        [
            "-c:v",
            "libx264",
            "-preset",
            request.encoding_mode.ffmpeg_preset(),
            "-crf",
        ]
        .iter()
        .map(ToString::to_string),
    );
    args.push(request.encoding_mode.crf().to_string());
    args.extend(
        ["-pix_fmt", "yuv420p", "-r"]
            .iter()
            .map(ToString::to_string),
    );
    args.push(request.fps.to_string());
    args.extend(
        [
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-ar",
            "48000",
            "-movflags",
            "+faststart",
        ]
        .iter()
        .map(ToString::to_string),
    );
    args.push(path_arg(&request.output));
    Ok(ExportPlan {
        preset: request.preset,
        width,
        height,
        fps: request.fps,
        encoding_mode: request.encoding_mode,
        bitrate_bps,
        output: request.output.clone(),
        ffmpeg_args: args,
    })
}

fn path_arg(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    pub job_id: Uuid,
    pub completed_frames: u64,
    pub total_frames: Option<u64>,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Progress parsing (pure, testable without an ffmpeg binary)
// ---------------------------------------------------------------------------

/// One parsed ffmpeg progress line.
///
/// Handles both `-progress pipe:1` key=value lines (`frame=`, `out_time_ms=`,
/// `out_time_us=`, `progress=continue|end`) and classic stderr lines
/// (`frame=  123 ...`). Returns `None` for lines that carry no progress.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FfmpegProgressEvent {
    pub frame: Option<u64>,
    pub out_time_ms: Option<u64>,
    pub finished: bool,
}

/// Parse a single ffmpeg output line into a progress event.
///
/// Pure function: no I/O, no process handling, safe to unit test.
///
/// Accepted shapes (all total over arbitrary input, never panics):
/// * `-progress` key=value lines: `frame=`, `out_time_ms=`, `out_time_us=`,
///   `progress=continue|end` (key matched ASCII case-insensitively so
///   `PROGRESS=END` also terminates);
/// * classic stderr status lines: `frame=  123 ... time=00:00:04.00 ...`.
#[must_use]
pub fn parse_ffmpeg_progress_line(line: &str) -> Option<FfmpegProgressEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some((key, value)) = trimmed.split_once('=')
        && key.trim().eq_ignore_ascii_case("progress")
    {
        return Some(FfmpegProgressEvent {
            frame: None,
            out_time_ms: None,
            finished: value.trim().eq_ignore_ascii_case("end"),
        });
    }
    if let Some(frame) = parse_ffmpeg_frame_count(trimmed) {
        return Some(FfmpegProgressEvent {
            frame: Some(frame),
            out_time_ms: parse_ffmpeg_out_time_ms(trimmed),
            finished: false,
        });
    }
    if let Some(out_time_ms) = parse_ffmpeg_out_time_ms(trimmed) {
        return Some(FfmpegProgressEvent {
            frame: None,
            out_time_ms: Some(out_time_ms),
            finished: false,
        });
    }
    None
}

/// Parse `frame=123` / `frame=  123` (with or without surrounding text) from
/// an ffmpeg stderr status line or a `-progress` key=value line. Classic
/// stderr pads the value with spaces (`frame=  240 fps= 60 ...`), so a bare
/// `frame=` token falls back to the next whitespace-separated token.
///
/// The `frame=` key matches ASCII case-insensitively; values must be plain
/// base-10 `u64` (negative, decimal, `N/A`, and overflowing values yield
/// `None` rather than a wrapped/clamped frame).
#[must_use]
pub fn parse_ffmpeg_frame_count(line: &str) -> Option<u64> {
    let mut tokens = line.split_whitespace().peekable();
    while let Some(token) = tokens.next() {
        if token.len() >= 6 && token[..6].eq_ignore_ascii_case("frame=") {
            let raw = token[6..].trim();
            if !raw.is_empty() {
                if let Ok(frame) = raw.parse::<u64>() {
                    return Some(frame);
                }
            } else if let Some(next) = tokens.peek()
                && let Ok(frame) = next.trim().parse::<u64>()
            {
                return Some(frame);
            }
        }
    }
    None
}

/// Parse `out_time_ms=` / `out_time_us=` progress keys, or `time=HH:MM:SS.xx`
/// stderr timestamps, into milliseconds.
///
/// `out_time_*` keys match ASCII case-insensitively and clamp negatives to
/// zero; `time=` timestamps accept `HH:MM:SS.cc` and `MM:SS.cc`, clamp
/// leading-`-` (pre-start) timestamps to zero, and saturate instead of
/// overflowing on absurd hour counts.
#[must_use]
pub fn parse_ffmpeg_out_time_ms(line: &str) -> Option<u64> {
    let trimmed = line.trim();
    if let Some((key, value)) = trimmed.split_once('=')
        && key.trim().eq_ignore_ascii_case("out_time_ms")
        && !key.trim().contains(' ')
        && !key.trim().contains('\t')
    {
        return value.trim().parse::<i64>().ok().map(|v| v.max(0) as u64);
    }
    if let Some((key, value)) = trimmed.split_once('=')
        && key.trim().eq_ignore_ascii_case("out_time_us")
        && !key.trim().contains(' ')
        && !key.trim().contains('\t')
    {
        return value
            .trim()
            .parse::<i64>()
            .ok()
            .map(|v| (v.max(0) / 1000) as u64);
    }
    for token in line.split_whitespace() {
        if token.len() >= 5
            && token[..5].eq_ignore_ascii_case("time=")
            && let Some(ms) = parse_ffmpeg_time_token(&token[5..])
        {
            return Some(ms);
        }
    }
    None
}

#[must_use]
pub fn is_ffmpeg_progress_end(line: &str) -> bool {
    line.trim().eq_ignore_ascii_case("progress=end")
}

fn parse_ffmpeg_time_token(token: &str) -> Option<u64> {
    // Expected `HH:MM:SS.cc`; also tolerates `MM:SS.cc`. A leading `-`
    // (ffmpeg can emit negative pre-start timestamps) clamps to zero instead
    // of reporting an absolute value. Saturating arithmetic keeps fuzz inputs
    // like `9999999999:00:00` total instead of panicking in debug builds.
    let token = token.trim();
    if token.eq_ignore_ascii_case("n/a") || token.eq_ignore_ascii_case("-n/a") {
        return None;
    }
    let (negative, token) = match token.strip_prefix('-') {
        Some(rest) => (true, rest.trim()),
        None => (false, token),
    };
    if token.is_empty() {
        return None;
    }
    let mut parts = token.split(':');
    let (hours, minutes, seconds) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(h), Some(m), Some(s), None) => (
            h.trim().parse::<u64>().ok()?,
            m.trim().parse::<u64>().ok()?,
            s.trim().parse::<f64>().ok()?,
        ),
        (Some(m), Some(s), None, _) => (
            0,
            m.trim().parse::<u64>().ok()?,
            s.trim().parse::<f64>().ok()?,
        ),
        _ => return None,
    };
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    if negative {
        return Some(0);
    }
    let millis = (seconds * 1000.0).round() as u64;
    Some(
        hours
            .saturating_mul(3_600_000)
            .saturating_add(minutes.saturating_mul(60_000))
            .saturating_add(millis),
    )
}

// ---------------------------------------------------------------------------
// Cancellation (jobs-style cooperative token)
// ---------------------------------------------------------------------------

/// Cooperative cancellation token for export jobs.
///
/// Mirrors `kiri-jobs::Job::request_cancel` semantics without taking a
/// dependency on the jobs crate: the runner polls [`Self::is_cancelled`]
/// between progress lines and kills the ffmpeg child on request. Share via
/// [`Clone`] across the UI thread and the worker thread.
#[derive(Debug, Clone, Default)]
pub struct ExportCancelToken {
    cancelled: Arc<AtomicBool>,
}

impl ExportCancelToken {
    #[must_use]
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------
// Execution runner (blocking; call from a worker thread / Tauri async wrapper)
// ---------------------------------------------------------------------------

/// Outcome of a finished ffmpeg run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOutcome {
    pub job_id: Uuid,
    pub output: PathBuf,
    pub completed_frames: u64,
    pub finished: bool,
}

/// Build the `ffmpeg` command for a plan. `ffmpeg_bin` is usually `"ffmpeg"`
/// (resolved via `PATH`); tests inject a stub binary.
#[must_use]
pub fn ffmpeg_command(plan: &ExportPlan, ffmpeg_bin: &str) -> std::process::Command {
    let mut command = std::process::Command::new(ffmpeg_bin);
    command.args(&plan.ffmpeg_args);
    command
}

/// Execute an export plan by spawning ffmpeg, streaming progress, and
/// honouring cooperative cancellation.
///
/// * `total_frames` is informational (e.g. `duration_ms * fps / 1000`) and is
///   echoed back in progress callbacks; pass `None` when unknown.
/// * `on_progress` is invoked for every parsed progress line.
/// * If `cancel` is already cancelled on entry, no process is spawned and
///   [`ExportError::Cancelled`] is returned immediately.
/// * On cancellation mid-run the child is killed and `Cancelled` is returned.
///   A watcher thread also kills silent children (no stderr output), so
///   cancellation never blocks on a quiet ffmpeg.
/// * Stdout is drained on a helper thread: ffmpeg writes diagnostics to
///   stderr, but a piped-but-unread stdout would eventually fill its pipe
///   buffer and deadlock the child.
/// * A drop guard kills the child on unwind (e.g. an `on_progress` panic),
///   so a failed callback can never leak a running ffmpeg.
/// * On non-zero exit, [`ExportError::FfmpegFailed`] carries the exit status
///   plus the last stderr lines for diagnostics.
///
/// Blocking: run on a worker thread, never on the Tauri UI thread.
pub fn run_export_blocking(
    plan: &ExportPlan,
    ffmpeg_bin: &str,
    total_frames: Option<u64>,
    cancel: Option<&ExportCancelToken>,
    mut on_progress: impl FnMut(ExportProgress),
) -> Result<ExportOutcome, ExportError> {
    run_export_inner(plan, ffmpeg_bin, total_frames, cancel, &mut on_progress)
}

/// RAII guard: kills (and reaps) the shared child unless disarmed.
/// Guarantees no ffmpeg zombie when `on_progress` panics or an early
/// `return` forgets to kill. Poison-tolerant: still reaps through
/// `into_inner` when the mutex is poisoned.
struct SharedChildGuard {
    shared: Option<std::sync::Arc<std::sync::Mutex<std::process::Child>>>,
}

impl SharedChildGuard {
    fn disarm(&mut self) {
        self.shared.take();
    }
}

impl Drop for SharedChildGuard {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.take() {
            let mut guard = shared.lock().unwrap_or_else(|err| err.into_inner());
            let _ = guard.kill();
            let _ = guard.wait();
        }
    }
}

fn lock_child(
    shared: &std::sync::Arc<std::sync::Mutex<std::process::Child>>,
) -> std::sync::MutexGuard<'_, std::process::Child> {
    shared.lock().unwrap_or_else(|err| err.into_inner())
}

fn run_export_inner(
    plan: &ExportPlan,
    ffmpeg_bin: &str,
    total_frames: Option<u64>,
    cancel: Option<&ExportCancelToken>,
    on_progress: &mut dyn FnMut(ExportProgress),
) -> Result<ExportOutcome, ExportError> {
    use std::io::BufRead as _;

    if cancel.is_some_and(|token| token.is_cancelled()) {
        return Err(ExportError::Cancelled);
    }
    let job_id = Uuid::new_v4();
    let mut child = std::process::Command::new(ffmpeg_bin)
        .args(&plan.ffmpeg_args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    // Drain stdout so a chatty child can never block on a full pipe.
    // Progress is parsed from stderr; stdout bytes are intentionally dropped.
    let stdout_handle = child.stdout.take().map(|stdout| {
        std::thread::spawn(move || {
            use std::io::{Read as _, Write as _};
            let mut sink = std::io::sink();
            let mut reader = std::io::BufReader::new(stdout);
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let _ = sink.write_all(&buf[..n]);
                    }
                }
            }
        })
    });

    // Watcher kills a silent child (blocked with no stderr lines) promptly.
    // Shares the child through a mutex; the main thread owns stderr reading
    // and the final `wait`, the watcher only kills. Poison-tolerant so a
    // panicking holder can never wedge the other thread.
    let cancel_owned = cancel.cloned();
    let shared = std::sync::Arc::new(std::sync::Mutex::new(child));
    let watcher_shared = std::sync::Arc::clone(&shared);
    let watcher_handle = std::thread::spawn(move || {
        if cancel_owned.as_ref().is_none() {
            return;
        }
        loop {
            if cancel_owned
                .as_ref()
                .is_some_and(|token| token.is_cancelled())
            {
                let mut guard = watcher_shared.lock().unwrap_or_else(|err| err.into_inner());
                let _ = guard.kill();
                break;
            }
            let exited = {
                let mut guard = watcher_shared.lock().unwrap_or_else(|err| err.into_inner());
                guard
                    .try_wait()
                    .map(|status| status.is_some())
                    .unwrap_or(true)
            };
            if exited {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });

    let mut shared_guard = SharedChildGuard {
        shared: Some(std::sync::Arc::clone(&shared)),
    };

    let stderr = lock_child(&shared).stderr.take();
    let mut completed_frames: u64 = 0;
    let mut tail: std::collections::VecDeque<String> = std::collections::VecDeque::new();

    if let Some(stderr) = stderr {
        let reader = std::io::BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            if cancel.is_some_and(|token| token.is_cancelled()) {
                {
                    let mut guard = lock_child(&shared);
                    let _ = guard.kill();
                    let _ = guard.wait();
                }
                shared_guard.disarm();
                let _ = watcher_handle.join();
                if let Some(handle) = stdout_handle {
                    let _ = handle.join();
                }
                return Err(ExportError::Cancelled);
            }
            if tail.len() >= FFMPEG_TAIL_LINES {
                tail.pop_front();
            }
            tail.push_back(line.clone());
            if let Some(event) = parse_ffmpeg_progress_line(&line) {
                if let Some(frame) = event.frame {
                    completed_frames = completed_frames.max(frame);
                }
                if event.finished {
                    break;
                }
                on_progress(ExportProgress {
                    job_id,
                    completed_frames,
                    total_frames,
                    message: line.clone(),
                });
            }
        }
    }

    let status = lock_child(&shared).wait()?;
    // Disarm: the child has been reaped; the Drop guard must not kill again.
    shared_guard.disarm();
    let _ = watcher_handle.join();
    if let Some(handle) = stdout_handle {
        let _ = handle.join();
    }

    if cancel.is_some_and(|token| token.is_cancelled()) {
        return Err(ExportError::Cancelled);
    }
    if status.success() {
        Ok(ExportOutcome {
            job_id,
            output: plan.output.clone(),
            completed_frames,
            finished: true,
        })
    } else {
        Err(ExportError::FfmpegFailed {
            status: status
                .code()
                .map_or_else(|| "signal".into(), |c| c.to_string()),
            tail: tail.into_iter().collect::<Vec<_>>().join("\n"),
        })
    }
}

// ---------------------------------------------------------------------------
// Headless scene math: the preview==export contract (Phase 2 gate)
// ---------------------------------------------------------------------------

/// Headless scene math shared by the Pixi preview renderer and this
/// exporter.
///
/// **preview==export contract (authoritative):** for every golden scene, the
/// preview and the exported MP4 must produce identical frames. A future Rust
/// preview renderer implements this contract exactly as follows — no
/// duplicated constants, no renderer-local tweaks:
///
/// 1. Resolve the per-depth zoom factor with
///    [`scene_math::zoom_scale_for_depth`] only (Recordly `ZOOM_DEPTH_SCALES`).
/// 2. Resolve the per-frame camera target (scale + normalized focus +
///    0..1 progress) from the timeline (zoom regions, cursor-follow). The
///    smoothing that turns targets into applied frames must be
///    deterministic: either classic mode (snap, `applied == projected`) or a
///    content-time spring (advance by media `delta_ms`, never wall-clock),
///    so the same media timestamp always yields the same frame on any
///    machine and at any render speed.
/// 3. Project the target to stage space with
///    [`scene_math::compute_zoom_transform`] only — a byte-for-byte port of
///    Recordly `zoomTransform.ts::computeZoomTransform` (same guards, same
///    clamp order, same `f64` arithmetic). Apply the resulting
///    [`scene_math::AppliedTransform`] to the camera container
///    (`scale`, `x`, `y`).
/// 4. Size every encoder surface with
///    [`scene_math::round_content_size`] (port of
///    `nativeStaticLayoutGeometry.ts`), which keeps aspect while rounding to
///    even dimensions required by `yuv420p`.
///
/// [`scene_math::focus_crop_rect`] is a legacy source-UV helper for simple
/// single-zoom cases; it is *not* part of the authoritative stage-space
/// path. New code must use [`scene_math::compute_zoom_transform`].
///
/// This module is intentionally headless (pure math, no GPU) so golden tests
/// can snapshot it on CI without a display.
pub mod scene_math {
    /// Zoom scale per depth level. Mirrors Recordly `ZOOM_DEPTH_SCALES`
    /// (`1: 1.25, 2: 1.5, 3: 1.8, 4: 2.2, 5: 3.5, 6: 5.0`).
    /// Out-of-range depths are clamped to 1..=6 so the function is total.
    #[must_use]
    pub fn zoom_scale_for_depth(depth: u32) -> f64 {
        match depth.clamp(1, 6) {
            1 => 1.25,
            2 => 1.5,
            3 => 1.8,
            4 => 2.2,
            5 => 3.5,
            _ => 5.0,
        }
    }

    /// Normalized crop rectangle (0..1 UV space) for a zoomed view.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct CropRect {
        pub x: f64,
        pub y: f64,
        pub width: f64,
        pub height: f64,
    }

    /// Deterministic crop for a zoom focus point.
    ///
    /// * `focus` is normalized (0..1); non-finite inputs are recentered.
    /// * The crop keeps the source aspect (`canvas_w`/`canvas_h` only guard
    ///   against degenerate zero sizes) and is clamped so it never leaves
    ///   the frame — the same clamp the preview applies.
    /// * Pure and deterministic: same inputs always yield the same rect.
    ///
    /// Legacy helper: prefer [`compute_zoom_transform`] for the
    /// authoritative preview==export path.
    #[must_use]
    pub fn focus_crop_rect(
        focus_x: f64,
        focus_y: f64,
        depth: u32,
        canvas_w: u32,
        canvas_h: u32,
    ) -> CropRect {
        let _ = (canvas_w.max(1), canvas_h.max(1));
        let scale = zoom_scale_for_depth(depth);
        let (w, h) = (1.0 / scale, 1.0 / scale);
        let cx = if focus_x.is_finite() {
            focus_x.clamp(0.0, 1.0)
        } else {
            0.5
        };
        let cy = if focus_y.is_finite() {
            focus_y.clamp(0.0, 1.0)
        } else {
            0.5
        };
        CropRect {
            x: (cx - w / 2.0).clamp(0.0, 1.0 - w),
            y: (cy - h / 2.0).clamp(0.0, 1.0 - h),
            width: w,
            height: h,
        }
    }

    /// Stage-space camera transform. Exact port of Recordly
    /// `zoomTransform.ts::computeZoomTransform`: `scale` scales the camera
    /// container, (`x`, `y`) positions it.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct AppliedTransform {
        pub scale: f64,
        pub x: f64,
        pub y: f64,
    }

    /// Project a zoom target to stage space.
    ///
    /// Ports `computeZoomTransform` verbatim:
    /// * degenerate `stage`/`mask` sizes yield identity (`scale 1, x 0, y 0`);
    /// * `zoom_progress` is clamped to `0..1`;
    /// * `scale = 1 + (zoom_scale - 1) * progress`;
    /// * `final = stage_center - focus_stage_px * zoom_scale`,
    ///   `applied = final * progress`, where `focus_stage_px = mask_origin +
    ///   focus * mask_size` (focus normalized to the mask).
    ///
    /// Non-finite `focus`/`zoom_scale`/`progress` fall back to identity
    /// rather than propagating NaN into the renderer. Pure and total.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn compute_zoom_transform(
        stage_w: f64,
        stage_h: f64,
        mask_x: f64,
        mask_y: f64,
        mask_w: f64,
        mask_h: f64,
        zoom_scale: f64,
        zoom_progress: f64,
        focus_x: f64,
        focus_y: f64,
    ) -> AppliedTransform {
        if !(stage_w.is_finite()
            && stage_h.is_finite()
            && mask_w.is_finite()
            && mask_h.is_finite()
            && mask_x.is_finite()
            && mask_y.is_finite()
            && zoom_scale.is_finite()
            && zoom_progress.is_finite()
            && focus_x.is_finite()
            && focus_y.is_finite())
        {
            return AppliedTransform {
                scale: 1.0,
                x: 0.0,
                y: 0.0,
            };
        }
        if stage_w <= 0.0 || stage_h <= 0.0 || mask_w <= 0.0 || mask_h <= 0.0 {
            return AppliedTransform {
                scale: 1.0,
                x: 0.0,
                y: 0.0,
            };
        }
        let progress = zoom_progress.clamp(0.0, 1.0);
        let focus_stage_x = mask_x + focus_x * mask_w;
        let focus_stage_y = mask_y + focus_y * mask_h;
        let stage_center_x = stage_w / 2.0;
        let stage_center_y = stage_h / 2.0;
        let scale = 1.0 + (zoom_scale - 1.0) * progress;
        let final_x = stage_center_x - focus_stage_x * zoom_scale;
        let final_y = stage_center_y - focus_stage_y * zoom_scale;
        AppliedTransform {
            scale,
            x: final_x * progress,
            y: final_y * progress,
        }
    }

    /// Inverse of [`compute_zoom_transform`]: recover the normalized mask
    /// focus that produced a stage-space translation. Ports
    /// `computeFocusFromTransform` verbatim (degenerate inputs recenter).
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn focus_from_transform(
        stage_w: f64,
        stage_h: f64,
        mask_x: f64,
        mask_y: f64,
        mask_w: f64,
        mask_h: f64,
        zoom_scale: f64,
        x: f64,
        y: f64,
    ) -> (f64, f64) {
        if !(stage_w.is_finite()
            && stage_h.is_finite()
            && mask_w.is_finite()
            && mask_h.is_finite()
            && mask_x.is_finite()
            && mask_y.is_finite()
            && zoom_scale.is_finite()
            && x.is_finite()
            && y.is_finite())
            || stage_w <= 0.0
            || stage_h <= 0.0
            || mask_w <= 0.0
            || mask_h <= 0.0
            || zoom_scale <= 0.0
        {
            return (0.5, 0.5);
        }
        let stage_center_x = stage_w / 2.0;
        let stage_center_y = stage_h / 2.0;
        let focus_stage_x = (stage_center_x - x) / zoom_scale;
        let focus_stage_y = (stage_center_y - y) / zoom_scale;
        (
            (focus_stage_x - mask_x) / mask_w,
            (focus_stage_y - mask_y) / mask_h,
        )
    }

    fn even_floor(value: f64) -> u32 {
        if !value.is_finite() || value <= 0.0 {
            return 2;
        }
        let floored = (value / 2.0).floor() * 2.0;
        floored.clamp(2.0, f64::from(u32::MAX - 1)) as u32
    }

    fn even_round(value: f64) -> u32 {
        if !value.is_finite() || value <= 0.0 {
            return 2;
        }
        let rounded = (value / 2.0).round() * 2.0;
        rounded.clamp(2.0, f64::from(u32::MAX - 1)) as u32
    }

    fn clamp_even(value: f64, max: u32) -> u32 {
        let rounded = even_round(value);
        if rounded <= max {
            rounded
        } else {
            even_floor(f64::from(max))
        }
    }

    /// Encoder-safe content size. Exact port of Recordly
    /// `nativeStaticLayoutGeometry.ts::roundNativeStaticLayoutContentSize`:
    /// keeps the requested aspect while rounding to even dimensions
    /// (required by `yuv420p`), trying both the width-driven and
    /// height-driven candidates and keeping the one with the smaller aspect
    /// error (larger area on ties). Non-finite/non-positive inputs fall back
    /// to the even-floored inputs. Pure and total.
    #[must_use]
    pub fn round_content_size(width: u32, height: u32) -> (u32, u32) {
        let max_w = even_floor(f64::from(width.max(1)));
        let max_h = even_floor(f64::from(height.max(1)));
        let (fw, fh) = (f64::from(width), f64::from(height));
        if !fw.is_finite() || !fh.is_finite() || fw <= 0.0 || fh <= 0.0 {
            return (max_w, max_h);
        }
        let aspect = fw / fh;
        if !aspect.is_finite() || aspect <= 0.0 {
            return (max_w, max_h);
        }
        let from_width = (max_w, clamp_even(f64::from(max_w) / aspect, max_h));
        let from_height = (clamp_even(f64::from(max_h) * aspect, max_w), max_h);
        let width_error = ((f64::from(from_width.0) / f64::from(from_width.1)) - aspect).abs();
        let height_error = ((f64::from(from_height.0) / f64::from(from_height.1)) - aspect).abs();
        if height_error < width_error {
            from_height
        } else if width_error < height_error {
            from_width
        } else if from_height.0 as u64 * from_height.1 as u64
            >= from_width.0 as u64 * from_width.1 as u64
        {
            from_height
        } else {
            from_width
        }
    }
}

// ---------------------------------------------------------------------------
// Export backend policy (ports Recordly `backendPolicy.ts` ideas)
// ---------------------------------------------------------------------------

/// Export backend preference. Mirrors Recordly `ExportBackendPreference`.
/// This crate only executes the native ffmpeg path; the other variants exist
/// so route decisions can name the backend the caller asked for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportBackendPreference {
    #[default]
    Auto,
    Ffmpeg,
    Webcodecs,
}

/// Native runtime platform, normalized from a free-form hint.
/// Ports `normalizeLightningRuntimePlatform` (`win` → win32, `linux`,
/// `mac|iphone|ipad|ipod` → darwin, else unknown).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportPlatform {
    Darwin,
    Win32,
    Linux,
    #[default]
    Unknown,
}

/// Normalize a platform hint (`std::env::consts::OS`, user-agent fragment,
/// `win32`, …) to an [`ExportPlatform`]. Total over all inputs, including
/// `None`.
#[must_use]
pub fn normalize_export_platform(hint: Option<&str>) -> ExportPlatform {
    let Some(hint) = hint else {
        return ExportPlatform::Unknown;
    };
    let lower = hint.to_ascii_lowercase();
    if lower.contains("win") {
        ExportPlatform::Win32
    } else if lower.contains("linux") {
        ExportPlatform::Linux
    } else if lower.contains("mac")
        || lower.contains("iphone")
        || lower.contains("ipad")
        || lower.contains("ipod")
        || lower == "darwin"
    {
        ExportPlatform::Darwin
    } else {
        ExportPlatform::Unknown
    }
}

/// Currently selected export route. This crate always selects `ffmpeg`;
/// the decision log explains why (mirrors `planLightningExportRoutes`, which
/// keeps Windows `auto` on the stable streaming route while the native
/// static-layout probe stays disabled).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExportRoute {
    #[default]
    Ffmpeg,
    Webcodecs,
}

/// One route decision with machine-readable reasons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRouteDecision {
    pub route: ExportRoute,
    pub status: RouteStatus,
    pub reasons: Vec<String>,
}

/// Selection status for a route decision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteStatus {
    #[default]
    Selected,
    Fallback,
    Rejected,
}

/// Full route plan: the selected route plus the audit trail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRoutePlan {
    pub selected_route: ExportRoute,
    pub decisions: Vec<ExportRouteDecision>,
}

/// Plan the export route. Always selects native ffmpeg and records why:
/// deterministic local execution, no browser WebCodecs dependency, and the
/// native static-layout probe stays disabled (mirrors
/// `WINDOWS_AUTO_STATIC_LAYOUT_FIRST_ENABLED = false`).
#[must_use]
pub fn plan_export_route(
    preference: ExportBackendPreference,
    platform: ExportPlatform,
) -> ExportRoutePlan {
    let _ = (preference, platform);
    ExportRoutePlan {
        selected_route: ExportRoute::Ffmpeg,
        decisions: vec![
            ExportRouteDecision {
                route: ExportRoute::Ffmpeg,
                status: RouteStatus::Selected,
                reasons: vec!["native-ffmpeg-default".to_string()],
            },
            ExportRouteDecision {
                route: ExportRoute::Webcodecs,
                status: RouteStatus::Fallback,
                reasons: vec!["ffmpeg-unavailable-fallback".to_string()],
            },
        ],
    }
}

/// Default render backend for any future preview surface. Mirrors
/// `getDefaultLightningRenderBackend` (`"webgl"` stays the stable default).
#[must_use]
pub const fn default_render_backend() -> &'static str {
    "webgl"
}

// ---------------------------------------------------------------------------
// Silence detection (Phase 3, rule-based only — no models, no network)
// ---------------------------------------------------------------------------

/// A silent span over the source timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SilenceInterval {
    pub start_ms: i64,
    pub end_ms: i64,
}

/// Detect silent intervals from per-window PCM peak amplitudes.
///
/// * `peaks` holds one peak amplitude per window in `0.0..=1.0`.
/// * `peak_sample_ms` is the duration each peak covers (must be `> 0`).
/// * `threshold` is the silence ceiling; samples with `0.0 <= peak < threshold`
///   count as silent. Clamped to `0.0..=1.0`; non-finite and negative peaks
///   count as sound (never silence) so corrupt windows cannot create false
///   cuts.
/// * Only silent runs spanning at least `min_silence_ms` are reported.
///
/// Pure function over peak arrays: no decoding, no Whisper, no transcription,
/// no network. Factors `0`/`0 ms` inputs yield an empty result (total).
/// Index→millisecond math saturates so adversarial inputs cannot wrap.
#[must_use]
pub fn detect_silences(
    peaks: &[f32],
    peak_sample_ms: u64,
    threshold: f32,
    min_silence_ms: u64,
) -> Vec<SilenceInterval> {
    if peaks.is_empty() || peak_sample_ms == 0 || min_silence_ms == 0 {
        return Vec::new();
    }
    let ceiling = if threshold.is_finite() {
        threshold.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut intervals = Vec::new();
    let mut run_start: Option<usize> = None;
    for (index, peak) in peaks.iter().enumerate() {
        let silent = peak.is_finite() && *peak >= 0.0 && *peak < ceiling;
        if silent {
            if run_start.is_none() {
                run_start = Some(index);
            }
        } else if let Some(start) = run_start.take() {
            push_silence(&mut intervals, start, index, peak_sample_ms, min_silence_ms);
        }
    }
    if let Some(start) = run_start {
        push_silence(
            &mut intervals,
            start,
            peaks.len(),
            peak_sample_ms,
            min_silence_ms,
        );
    }
    intervals
}

fn push_silence(
    out: &mut Vec<SilenceInterval>,
    start_index: usize,
    end_index: usize,
    peak_sample_ms: u64,
    min_silence_ms: u64,
) {
    let start_ms = (start_index as u64).saturating_mul(peak_sample_ms);
    let end_ms = (end_index as u64).saturating_mul(peak_sample_ms);
    if end_ms.saturating_sub(start_ms) >= min_silence_ms {
        out.push(SilenceInterval {
            start_ms: i64::try_from(start_ms).unwrap_or(i64::MAX),
            end_ms: i64::try_from(end_ms).unwrap_or(i64::MAX),
        });
    }
}

/// Policy for turning detected silences into export skip ranges.
///
/// Rule-based only (no models): a detected silence is actually skipped when,
/// after keeping `padding_ms` of breathing room on each edge, at least
/// `min_skip_ms` remains. Padding prevents abrupt cuts; the minimum prevents
/// chopping tiny blips. Both fields are total: `0` disables that leg.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SilenceSkipPolicy {
    pub min_skip_ms: u64,
    pub padding_ms: u64,
}

impl Default for SilenceSkipPolicy {
    fn default() -> Self {
        Self {
            min_skip_ms: 400,
            padding_ms: 120,
        }
    }
}

impl SilenceSkipPolicy {
    #[must_use]
    pub fn new(min_skip_ms: u64, padding_ms: u64) -> Self {
        Self {
            min_skip_ms,
            padding_ms,
        }
    }
}

/// Plan which detected silences to skip under `policy`.
///
/// For each `interval` with `end_ms > start_ms`, the skippable core is
/// `[start_ms + padding, end_ms - padding]` (saturating, clamped to the
/// interval). Cores shorter than `min_skip_ms` are dropped. Output is sorted
/// by `start_ms`, never overlaps its source beyond shrinking, and is total
/// over malformed inputs (inverted/negative intervals are ignored, never
/// panic).
#[must_use]
pub fn plan_silence_skips(
    intervals: &[SilenceInterval],
    policy: SilenceSkipPolicy,
) -> Vec<SilenceInterval> {
    let mut out = Vec::new();
    for interval in intervals {
        if interval.end_ms <= interval.start_ms || interval.end_ms < 0 {
            continue;
        }
        let start = interval.start_ms.max(0);
        let end = interval.end_ms.max(0);
        if end <= start {
            continue;
        }
        let pad = policy.padding_ms.min(((end - start) as u64) / 2);
        let skip_start = start.saturating_add(pad as i64);
        let skip_end = end.saturating_sub(pad as i64);
        if skip_end <= skip_start {
            continue;
        }
        let core_ms = (skip_end - skip_start) as u64;
        if core_ms >= policy.min_skip_ms {
            out.push(SilenceInterval {
                start_ms: skip_start,
                end_ms: skip_end,
            });
        }
    }
    out.sort_by_key(|interval| interval.start_ms);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_math;

    fn request() -> ExportRequest {
        ExportRequest {
            project_id: Uuid::new_v4(),
            preset: ExportPreset::FullHd,
            fps: 60,
            encoding_mode: EncodingMode::Balanced,
            video_input: PathBuf::from("media/screen-0001.mp4"),
            audio_inputs: vec![PathBuf::from("media/microphone-0001.wav")],
            output: PathBuf::from("exports/walkthrough.mp4"),
        }
    }

    #[test]
    fn plan_is_deterministic_and_h264_aac() {
        let first = plan_export(&request()).unwrap();
        let second = plan_export(&request()).unwrap();
        assert_eq!(first.ffmpeg_args, second.ffmpeg_args);
        assert!(first.ffmpeg_args.contains(&"libx264".to_string()));
        assert!(first.ffmpeg_args.contains(&"aac".to_string()));
        assert!(first.ffmpeg_args.contains(&"+faststart".to_string()));
        assert_eq!((first.width, first.height), (1920, 1080));
    }

    #[test]
    fn invalid_fps_is_rejected() {
        let mut invalid = request();
        invalid.fps = 24 - 1;
        assert!(matches!(
            plan_export(&invalid).unwrap_err(),
            ExportError::Invalid(_)
        ));
        let mut invalid_rate = request();
        invalid_rate.fps = 24;
        assert!(plan_export(&invalid_rate).is_ok());
    }

    #[test]
    fn valid_frame_rates_match_recordly_policy() {
        assert!(is_valid_mp4_frame_rate(24));
        assert!(is_valid_mp4_frame_rate(30));
        assert!(is_valid_mp4_frame_rate(60));
        assert!(!is_valid_mp4_frame_rate(25));
        assert!(!is_valid_mp4_frame_rate(0));
    }

    #[test]
    fn non_mp4_output_is_rejected() {
        let mut bad = request();
        bad.output = PathBuf::from("exports/walkthrough.webm");
        assert!(matches!(
            plan_export(&bad).unwrap_err(),
            ExportError::Invalid(_)
        ));
    }

    #[test]
    fn encoding_mode_selects_ffmpeg_preset_and_crf() {
        for (mode, preset, crf) in [
            (EncodingMode::Fast, "veryfast", "20"),
            (EncodingMode::Balanced, "medium", "18"),
            (EncodingMode::Quality, "slow", "16"),
        ] {
            let mut req = request();
            req.encoding_mode = mode;
            let plan = plan_export(&req).unwrap();
            let flag = plan
                .ffmpeg_args
                .windows(2)
                .find(|w| w[0] == "-preset")
                .unwrap()[1]
                .clone();
            let crf_value = plan
                .ffmpeg_args
                .windows(2)
                .find(|w| w[0] == "-crf")
                .unwrap()[1]
                .clone();
            assert_eq!(flag, preset);
            assert_eq!(crf_value, crf);
        }
    }

    #[test]
    fn default_encoding_mode_keeps_golden_args_stable() {
        let plan = plan_export(&request()).unwrap();
        assert!(plan.ffmpeg_args.contains(&"medium".to_string()));
        assert!(plan.ffmpeg_args.contains(&"18".to_string()));
    }

    #[test]
    fn target_bitrate_follows_recordly_multipliers() {
        assert_eq!(
            target_bitrate_bps(ExportPreset::FullHd, 30, EncodingMode::Balanced),
            9_600_000
        );
        assert_eq!(
            target_bitrate_bps(ExportPreset::Hd, 30, EncodingMode::Fast),
            4_000_000
        );
        assert_eq!(
            target_bitrate_bps(ExportPreset::FullHd, 60, EncodingMode::Quality),
            16_970_563
        );
        // Floor holds for degenerate inputs.
        assert!(target_bitrate_bps(ExportPreset::Hd, 0, EncodingMode::Fast) >= 2_000_000);
    }

    #[test]
    fn old_requests_without_encoding_mode_still_parse() {
        let json = serde_json::json!({
            "projectId": Uuid::new_v4(),
            "preset": "720p",
            "fps": 30,
            "videoInput": "media/screen-0001.mp4",
            "audioInputs": [],
            "output": "exports/out.mp4",
        });
        let req: ExportRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.encoding_mode, EncodingMode::Balanced);
        assert!(plan_export(&req).is_ok());
    }

    #[test]
    fn progress_parses_progress_pipe_and_stderr_lines() {
        let frame = parse_ffmpeg_progress_line("frame=123").unwrap();
        assert_eq!(frame.frame, Some(123));
        let ms = parse_ffmpeg_progress_line("out_time_ms=2500000").unwrap();
        assert_eq!(ms.out_time_ms, Some(2_500_000));
        let us = parse_ffmpeg_progress_line("out_time_us=2500000").unwrap();
        assert_eq!(us.out_time_ms, Some(2500));
        let stderr = parse_ffmpeg_progress_line(
            "frame=  240 fps=60 q=28.0 size=    1024kB time=00:00:04.00 bitrate=1000kbits/s",
        )
        .unwrap();
        assert_eq!(stderr.frame, Some(240));
        assert_eq!(stderr.out_time_ms, Some(4000));
        let end = parse_ffmpeg_progress_line("progress=end").unwrap();
        assert!(end.finished);
        assert!(parse_ffmpeg_progress_line("hello").is_none());
        assert!(parse_ffmpeg_progress_line("").is_none());
        assert!(is_ffmpeg_progress_end("progress=end"));
        assert!(!is_ffmpeg_progress_end("progress=continue"));
    }

    #[test]
    fn cancellation_before_start_spawns_nothing() {
        let plan = plan_export(&request()).unwrap();
        let token = ExportCancelToken::new();
        token.cancel();
        assert!(token.is_cancelled());
        let result = run_export_blocking(&plan, "ffmpeg", None, Some(&token), |_| {});
        assert!(matches!(result, Err(ExportError::Cancelled)));
    }

    #[test]
    fn missing_ffmpeg_binary_surfaces_io_error() {
        let plan = plan_export(&request()).unwrap();
        let result = run_export_blocking(
            &plan,
            "kiri-definitely-missing-ffmpeg-bin",
            None,
            None,
            |_| {},
        );
        assert!(matches!(result, Err(ExportError::Io(_))));
    }

    #[test]
    fn scene_math_matches_recordly_depth_scales() {
        assert_eq!(scene_math::zoom_scale_for_depth(1), 1.25);
        assert_eq!(scene_math::zoom_scale_for_depth(2), 1.5);
        assert_eq!(scene_math::zoom_scale_for_depth(3), 1.8);
        assert_eq!(scene_math::zoom_scale_for_depth(4), 2.2);
        assert_eq!(scene_math::zoom_scale_for_depth(5), 3.5);
        assert_eq!(scene_math::zoom_scale_for_depth(6), 5.0);
        // Total over out-of-range depths.
        assert_eq!(scene_math::zoom_scale_for_depth(0), 1.25);
        assert_eq!(scene_math::zoom_scale_for_depth(99), 5.0);
    }

    #[test]
    fn scene_math_crop_is_deterministic_and_clamped() {
        let first = scene_math::focus_crop_rect(0.5, 0.5, 2, 1920, 1080);
        let second = scene_math::focus_crop_rect(0.5, 0.5, 2, 1920, 1080);
        assert_eq!(first, second);
        // Depth 2 -> scale 1.5 -> 2/3 crop centered.
        assert!((first.width - 2.0 / 3.0).abs() < 1e-9);
        let corner = scene_math::focus_crop_rect(0.0, 0.0, 6, 1920, 1080);
        assert_eq!((corner.x, corner.y), (0.0, 0.0));
        let far = scene_math::focus_crop_rect(9.0, -9.0, 2, 1920, 1080);
        assert!(far.x >= 0.0 && far.y >= 0.0);
        assert!(far.x + far.width <= 1.0 + 1e-9);
        assert!(far.y + far.height <= 1.0 + 1e-9);
        let nan = scene_math::focus_crop_rect(f64::NAN, f64::NAN, 2, 0, 0);
        assert!(nan.x.is_finite() && nan.y.is_finite());
    }

    #[test]
    fn silence_detection_reports_only_long_runs() {
        // 10 ms windows: loud, loud, 5x silent, loud.
        let peaks = [0.8, 0.9, 0.01, 0.02, 0.0, 0.01, 0.03, 0.7];
        let intervals = detect_silences(&peaks, 10, 0.1, 30);
        assert_eq!(
            intervals,
            vec![SilenceInterval {
                start_ms: 20,
                end_ms: 70
            }]
        );
        // Higher bar filters everything out.
        assert!(detect_silences(&peaks, 10, 0.1, 60).is_empty());
        // All loud -> no intervals; all silent -> one interval.
        assert!(detect_silences(&[0.9, 0.8], 10, 0.1, 10).is_empty());
        assert_eq!(
            detect_silences(&[0.0, 0.0, 0.0], 10, 0.1, 20),
            vec![SilenceInterval {
                start_ms: 0,
                end_ms: 30
            }]
        );
    }

    #[test]
    fn silence_detection_is_total_over_degenerate_inputs() {
        assert!(detect_silences(&[], 10, 0.1, 10).is_empty());
        assert!(detect_silences(&[0.0], 0, 0.1, 10).is_empty());
        assert!(detect_silences(&[0.0], 10, 0.1, 0).is_empty());
        // Non-finite peaks never count as silence.
        assert!(detect_silences(&[f32::NAN, f32::INFINITY], 10, 0.5, 10).is_empty());
    }
}
