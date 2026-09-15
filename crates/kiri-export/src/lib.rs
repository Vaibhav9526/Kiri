//! Deterministic local MP4 export planning (Phase 2 redo part 1).
//!
//! Recordly mapping: `src/lib/exporter/` (WebCodecs + mediabunny in the
//! browser, native compositor fast paths) becomes a Rust-owned pipeline:
//! the preview renderer and this exporter share the same scene math so that
//! preview and exported frames match for the golden scenes (Phase 2 gate).
//!
//! This slice owns presets, request validation, and deterministic ffmpeg
//! argument planning. Frame rendering and process execution follow in the
//! next slice. AI features (captions burn-in from transcription, auto-zoom)
//! stay out until their explicit phase.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("invalid export plan: {0}")]
    Invalid(String),
    #[error("export I/O failed: {0}")]
    Io(#[from] std::io::Error),
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub project_id: Uuid,
    pub preset: ExportPreset,
    pub fps: u32,
    pub video_input: PathBuf,
    pub audio_inputs: Vec<PathBuf>,
    pub output: PathBuf,
}

impl ExportRequest {
    pub fn validate(&self) -> Result<(), ExportError> {
        if self.fps != 30 && self.fps != 60 {
            return Err(ExportError::Invalid(format!(
                "unsupported export fps {}",
                self.fps
            )));
        }
        if self.video_input.as_os_str().is_empty() {
            return Err(ExportError::Invalid("video input is required".into()));
        }
        if self.output.as_os_str().is_empty() {
            return Err(ExportError::Invalid("export output is required".into()));
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
            "-map", "0:v:0", "-map", "1:a:0?", "-c:v", "libx264", "-preset", "medium", "-crf",
            "18", "-pix_fmt", "yuv420p", "-r",
        ]
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

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ExportRequest {
        ExportRequest {
            project_id: Uuid::new_v4(),
            preset: ExportPreset::FullHd,
            fps: 60,
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
        invalid.fps = 24;
        assert!(matches!(
            plan_export(&invalid).unwrap_err(),
            ExportError::Invalid(_)
        ));
    }
}
