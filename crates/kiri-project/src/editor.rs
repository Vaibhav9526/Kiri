//! Recordly-compatible editor state (`video-editor/types.ts` port).
//!
//! Phase 0 establishes the schema and normalization rules. Phase 2 builds
//! the timeline UI, Pixi preview, and MP4 export on top of it. AI-backed
//! fields (auto captions, transcript) are accepted on load but never
//! generated locally until their explicit phase.

use serde::{Deserialize, Serialize};

pub const EDITOR_SCHEMA_VERSION: u32 = 2;

fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ZoomMode {
    Auto,
    #[default]
    Manual,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoomFocus {
    pub cx: f64,
    pub cy: f64,
}

impl Default for ZoomFocus {
    fn default() -> Self {
        Self { cx: 0.5, cy: 0.5 }
    }
}

impl ZoomFocus {
    pub fn normalized(mut self) -> Self {
        self.cx = clamp01(self.cx);
        self.cy = clamp01(self.cy);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZoomRegion {
    pub id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    #[serde(default = "default_zoom_depth")]
    pub depth: u32,
    #[serde(default)]
    pub focus: ZoomFocus,
    #[serde(default)]
    pub mode: ZoomMode,
}

fn default_zoom_depth() -> u32 {
    2
}

impl ZoomRegion {
    pub fn normalized(mut self) -> Self {
        if self.end_ms < self.start_ms {
            std::mem::swap(&mut self.start_ms, &mut self.end_ms);
        }
        self.depth = self.depth.clamp(1, 6);
        self.focus = self.focus.normalized();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipRegion {
    pub id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    #[serde(default = "default_speed")]
    pub speed: f64,
    #[serde(default)]
    pub muted: bool,
}

fn default_speed() -> f64 {
    1.0
}

impl ClipRegion {
    pub fn normalized(mut self) -> Self {
        if self.end_ms < self.start_ms {
            std::mem::swap(&mut self.start_ms, &mut self.end_ms);
        }
        if !self.speed.is_finite() || self.speed <= 0.0 {
            self.speed = 1.0;
        }
        self.speed = self.speed.clamp(0.25, 4.0);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrimRegion {
    pub id: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeedRegion {
    pub id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub speed: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CropRegion {
    #[serde(default = "default_crop_x")]
    pub x: f64,
    #[serde(default = "default_crop_y")]
    pub y: f64,
    #[serde(default = "default_crop_size")]
    pub width: f64,
    #[serde(default = "default_crop_size")]
    pub height: f64,
}

fn default_crop_x() -> f64 {
    0.0
}
fn default_crop_y() -> f64 {
    0.0
}
fn default_crop_size() -> f64 {
    1.0
}

impl Default for CropRegion {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebcamOverlay {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub time_offset_ms: i64,
    #[serde(default = "default_true")]
    pub mirror: bool,
    #[serde(default)]
    pub crop: CropRegion,
    #[serde(default)]
    pub position_x: f64,
    #[serde(default)]
    pub position_y: f64,
    #[serde(default = "default_webcam_size")]
    pub size: f64,
    #[serde(default = "default_true")]
    pub react_to_zoom: bool,
    #[serde(default)]
    pub roundness: f64,
    #[serde(default)]
    pub shadow: f64,
}

fn default_true() -> bool {
    true
}

fn default_webcam_size() -> f64 {
    0.25
}

impl Default for WebcamOverlay {
    fn default() -> Self {
        Self {
            enabled: false,
            source_path: None,
            time_offset_ms: 0,
            mirror: true,
            crop: CropRegion::default(),
            position_x: 0.85,
            position_y: 0.85,
            size: 0.25,
            react_to_zoom: true,
            roundness: 0.2,
            shadow: 0.4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionWord {
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionCue {
    pub id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub words: Vec<CaptionWord>,
}

/// Appearance shared by Pixi preview and the exporter. Preview and export
/// must render identical frames for the golden scenes (Phase 2 gate).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Appearance {
    #[serde(default = "default_background")]
    pub background: String,
    #[serde(default)]
    pub padding: f64,
    #[serde(default = "default_radius")]
    pub border_radius: f64,
    #[serde(default)]
    pub shadow: f64,
    #[serde(default)]
    pub aspect_ratio: Option<String>,
}

fn default_background() -> String {
    "#0f1115".into()
}

fn default_radius() -> f64 {
    12.0
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            background: default_background(),
            padding: 48.0,
            border_radius: default_radius(),
            shadow: 0.35,
            aspect_ratio: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorState {
    #[serde(default = "default_editor_version")]
    pub version: u32,
    #[serde(default)]
    pub appearance: Appearance,
    #[serde(default)]
    pub zooms: Vec<ZoomRegion>,
    #[serde(default)]
    pub clips: Vec<ClipRegion>,
    #[serde(default)]
    pub trims: Vec<TrimRegion>,
    #[serde(default)]
    pub speeds: Vec<SpeedRegion>,
    #[serde(default)]
    pub captions: Vec<CaptionCue>,
    #[serde(default)]
    pub webcam: WebcamOverlay,
}

fn default_editor_version() -> u32 {
    EDITOR_SCHEMA_VERSION
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            version: EDITOR_SCHEMA_VERSION,
            appearance: Appearance::default(),
            zooms: vec![],
            clips: vec![],
            trims: vec![],
            speeds: vec![],
            captions: vec![],
            webcam: WebcamOverlay::default(),
        }
    }
}

impl EditorState {
    /// Clamp every field into its valid range and drop nothing silently
    /// except inverted ranges, which are repaired by swapping.
    pub fn normalized(mut self) -> Self {
        self.version = EDITOR_SCHEMA_VERSION;
        self.zooms = self.zooms.into_iter().map(ZoomRegion::normalized).collect();
        self.clips = self.clips.into_iter().map(ClipRegion::normalized).collect();
        self.webcam.position_x = clamp01(self.webcam.position_x);
        self.webcam.position_y = clamp01(self.webcam.position_y);
        self.webcam.size = self.webcam.size.clamp(0.05, 1.0);
        self.webcam.roundness = self.webcam.roundness.clamp(0.0, 1.0);
        self.webcam.shadow = self.webcam.shadow.clamp(0.0, 1.0);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_normalization_repairs_range_and_depth() {
        let region = ZoomRegion {
            id: "z1".into(),
            start_ms: 900,
            end_ms: 100,
            depth: 99,
            focus: ZoomFocus { cx: 9.0, cy: -2.0 },
            mode: ZoomMode::Auto,
        }
        .normalized();
        assert_eq!((region.start_ms, region.end_ms), (100, 900));
        assert_eq!(region.depth, 6);
        assert_eq!(region.focus, ZoomFocus { cx: 1.0, cy: 0.0 }.normalized());
    }

    #[test]
    fn editor_state_defaults_are_valid_and_versioned() {
        let state = EditorState::default().normalized();
        assert_eq!(state.version, EDITOR_SCHEMA_VERSION);
        assert!(state.zooms.is_empty());
        assert!(state.webcam.mirror);
    }
}
