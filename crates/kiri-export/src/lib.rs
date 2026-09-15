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
}

/// Target delivery bitrate in bits per second for a preset/fps/mode triple.
/// Pure function so golden tests can snapshot export policy byte-for-byte.
#[must_use]
pub fn target_bitrate_bps(preset: ExportPreset, fps: u32, mode: EncodingMode) -> u32 {
    let fps_factor = (f64::from(fps.max(1)) / 30.0).max(1.0).sqrt();
    let target = f64::from(preset.base_bitrate_bps()) * fps_factor * mode.bitrate_multiplier();
    (target.round() as u32).max(2_000_000)
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
        if self.video_input.as_os_str().is_empty() {
            return Err(ExportError::Invalid("video input is required".into()));
        }
        if self.output.as_os_str().is_empty() {
            return Err(ExportError::Invalid("export output is required".into()));
        }
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
        if self.audio_inputs.len() > 8 {
            return Err(ExportError::Invalid(
                "at most 8 audio inputs are supported".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportPlan {
    pub preset: ExportPreset,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub output: PathBuf,
    pub ffmpeg_args: Vec<String>,
}

/// Builds a deterministic ffmpeg argument list: H.264 video + AAC audio in
/// MP4, `+faststart` for progressive playback. Argument order is fixed so
/// golden tests can snapshot the plan byte-for-byte.
///
/// `encoding_mode` selects the x264 preset/CRF pair:
/// fast = veryfast/crf20, balanced = medium/crf18, quality = slow/crf16.
pub fn plan_export(request: &ExportRequest) -> Result<ExportPlan, ExportError> {
    request.validate()?;
    let (width, height) = request.preset.dimensions();
    let mut args = vec!["-y".into(), "-i".into(), path_arg(&request.video_input)];
    for audio in &request.audio_inputs {
        args.push("-i".into());
        args.push(path_arg(audio));
    }
    args.extend(
        [
            "-map",
            "0:v:0",
            "-map",
            "1:a:0?",
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
#[must_use]
pub fn parse_ffmpeg_progress_line(line: &str) -> Option<FfmpegProgressEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(value) = trimmed
        .strip_prefix("progress=")
        .map(str::trim)
        .map(|v| v.to_ascii_lowercase())
    {
        return Some(FfmpegProgressEvent {
            frame: None,
            out_time_ms: None,
            finished: value == "end",
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
#[must_use]
pub fn parse_ffmpeg_frame_count(line: &str) -> Option<u64> {
    let mut tokens = line.split_whitespace().peekable();
    while let Some(token) = tokens.next() {
        if let Some(raw) = token.strip_prefix("frame=") {
            let raw = raw.trim();
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
#[must_use]
pub fn parse_ffmpeg_out_time_ms(line: &str) -> Option<u64> {
    let trimmed = line.trim();
    if let Some(raw) = trimmed.strip_prefix("out_time_ms=") {
        return raw.trim().parse::<i64>().ok().map(|v| v.max(0) as u64);
    }
    if let Some(raw) = trimmed.strip_prefix("out_time_us=") {
        return raw
            .trim()
            .parse::<i64>()
            .ok()
            .map(|v| (v.max(0) / 1000) as u64);
    }
    for token in line.split_whitespace() {
        if let Some(raw) = token.strip_prefix("time=")
            && let Some(ms) = parse_ffmpeg_time_token(raw)
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
    // Expected `HH:MM:SS.cc`; also tolerates `MM:SS.cc` and `-HH:MM:SS`.
    let token = token.trim().trim_start_matches('-');
    if token.eq_ignore_ascii_case("n/a") {
        return None;
    }
    let mut parts = token.split(':');
    let (hours, minutes, seconds) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(h), Some(m), Some(s), None) => (
            h.parse::<u64>().ok()?,
            m.parse::<u64>().ok()?,
            s.parse::<f64>().ok()?,
        ),
        (Some(m), Some(s), None, _) => (0, m.parse::<u64>().ok()?, s.parse::<f64>().ok()?),
        _ => return None,
    };
    if !(seconds.is_finite()) || seconds < 0.0 {
        return None;
    }
    Some(hours * 3_600_000 + minutes * 60_000 + (seconds * 1000.0).round() as u64)
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

    let stderr = child.stderr.take();
    let mut completed_frames: u64 = 0;
    let mut tail: Vec<String> = Vec::new();

    if let Some(stderr) = stderr {
        let reader = std::io::BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            if cancel.is_some_and(|token| token.is_cancelled()) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ExportError::Cancelled);
            }
            if tail.len() >= 20 {
                tail.remove(0);
            }
            tail.push(line.clone());
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

    let status = child.wait()?;
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
            tail: tail.join("\n"),
        })
    }
}

// ---------------------------------------------------------------------------
// Headless scene math: the preview==export contract (Phase 2 gate)
// ---------------------------------------------------------------------------

/// Headless scene math shared by the Pixi preview renderer and this
/// exporter.
///
/// **preview==export contract:** for every golden scene, the preview and the
/// exported MP4 must produce identical frames. Both sides must derive zoom
/// scale and crop rectangles from [`scene_math::zoom_scale_for_depth`] and
/// [`scene_math::focus_crop_rect`] only — no duplicated constants, no
/// renderer-local tweaks. This module is intentionally headless (pure math,
/// no GPU) so golden tests can snapshot it on CI without a display.
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
/// * `threshold` is the silence ceiling; samples with `peak < threshold`
///   count as silent. Clamped to `0.0..=1.0`; non-finite peaks count as
///   sound (never silence) so corrupt windows cannot create false cuts.
/// * Only silent runs spanning at least `min_silence_ms` are reported.
///
/// Pure function over peak arrays: no decoding, no Whisper, no transcription,
/// no network. Factors `0`/`0 ms` inputs yield an empty result (total).
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
        let silent = peak.is_finite() && *peak < ceiling;
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
    let start_ms = start_index as u64 * peak_sample_ms;
    let end_ms = end_index as u64 * peak_sample_ms;
    if end_ms.saturating_sub(start_ms) >= min_silence_ms {
        out.push(SilenceInterval {
            start_ms: start_ms as i64,
            end_ms: end_ms as i64,
        });
    }
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
