//! Recordly-compatible editor state (`video-editor/types.ts` port).
//!
//! Phase 0 establishes the schema and normalization rules. Phase 2 builds
//! the timeline UI, Pixi preview, and MP4 export on top of it. AI-backed
//! fields (auto captions, transcript) are accepted on load but never
//! generated locally until their explicit phase.
//!
//! Phase 2 additions (this slice): undo-safe clip-region ops
//! (add/move/split/trim with validation), annotation regions
//! (text/figure/blur) and audio regions. All mutating ops validate first
//! and leave the state untouched on error so the UI can push undo entries
//! only for successful ops.
//!
//! Phase 3 additions (rule-based only, no models): [`suggest_zoom_regions`]
//! derives [`ZoomRegion`]s from cursor telemetry with the same click-cluster
//! heuristic as Recordly `zoomSuggestionUtils.ts`. No ML, no network.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const EDITOR_SCHEMA_VERSION: u32 = 2;

/// Max gap between consecutive clicks before they start separate zoom
/// clusters. Mirrors Recordly `CLICK_CLUSTER_MERGE_GAP_MS`.
pub const CLICK_CLUSTER_MERGE_GAP_MS: i64 = 2500;
/// Padding added before the first click and after the last click in a
/// cluster. Mirrors Recordly `CLICK_CLUSTER_PAD_MS`.
pub const CLICK_CLUSTER_PAD_MS: i64 = 500;
/// Depth used for heuristic zoom suggestions. Mirrors Recordly
/// `DEFAULT_AUTO_ZOOM_DEPTH`.
pub const SUGGESTED_ZOOM_DEPTH: u32 = 2;

fn clamp01(value: f64) -> f64 {
    if value.is_nan() {
        return 0.5;
    }
    value.clamp(0.0, 1.0)
}

fn clamp_percent(value: f64) -> f64 {
    if value.is_nan() {
        return 50.0;
    }
    value.clamp(0.0, 100.0)
}

fn clamp_finite_or(value: f64, min: f64, max: f64, fallback: f64) -> f64 {
    if !value.is_finite() {
        return fallback;
    }
    value.clamp(min, max)
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
    /// Timeline-independent source in-point. `None` means "reads from
    /// `start_ms`" (true while everything before the clip plays at 1x).
    /// Splitting or left-trimming a sped-up clip moves the source in-point
    /// without moving the clip; moving a clip keeps it. Mirrors Recordly
    /// `sourceStartMs`.
    #[serde(default)]
    pub source_start_ms: Option<i64>,
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
        if let Some(source) = self.source_start_ms
            && source < 0
        {
            self.source_start_ms = None;
        }
        self
    }

    /// Where the clip reads from in the recording.
    #[must_use]
    pub fn source_start(&self) -> i64 {
        self.source_start_ms.unwrap_or(self.start_ms).max(0)
    }

    /// Source out-point derived from display duration and speed. Mirrors
    /// Recordly `getClipSourceEndMs`.
    #[must_use]
    pub fn source_end(&self) -> i64 {
        let display = (self.end_ms - self.start_ms).max(0) as f64;
        let speed = if self.speed.is_finite() && self.speed > 0.0 {
            self.speed
        } else {
            1.0
        };
        self.source_start() + (display * speed).round() as i64
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

// ---------------------------------------------------------------------------
// Annotations (Phase 2): text / figure / blur overlays
// ---------------------------------------------------------------------------

/// Annotation overlay kind. Mirrors Recordly `AnnotationType` restricted to
/// the natively rendered set (image overlays arrive in a later slice).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnnotationKind {
    #[default]
    Text,
    Figure,
    Blur,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArrowDirection {
    Up,
    Down,
    Left,
    #[default]
    Right,
    #[serde(rename = "up-right")]
    UpRight,
    #[serde(rename = "up-left")]
    UpLeft,
    #[serde(rename = "down-right")]
    DownRight,
    #[serde(rename = "down-left")]
    DownLeft,
}

/// Position in percent of canvas (0..100). Mirrors Recordly
/// `DEFAULT_ANNOTATION_POSITION = { x: 50, y: 50 }`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationPosition {
    pub x: f64,
    pub y: f64,
}

impl Default for AnnotationPosition {
    fn default() -> Self {
        Self { x: 50.0, y: 50.0 }
    }
}

/// Size in percent of canvas. Mirrors Recordly
/// `DEFAULT_ANNOTATION_SIZE = { width: 30, height: 20 }`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationSize {
    pub width: f64,
    pub height: f64,
}

impl Default for AnnotationSize {
    fn default() -> Self {
        Self {
            width: 30.0,
            height: 20.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationTextStyle {
    #[serde(default = "default_annotation_color")]
    pub color: String,
    #[serde(default = "default_annotation_background")]
    pub background_color: String,
    #[serde(default = "default_annotation_font_size")]
    pub font_size: f64,
    #[serde(default = "default_annotation_font_family")]
    pub font_family: String,
    #[serde(default = "default_true")]
    pub bold: bool,
    #[serde(default = "default_annotation_radius")]
    pub border_radius: f64,
}

fn default_annotation_color() -> String {
    "#ffffff".into()
}
fn default_annotation_background() -> String {
    "transparent".into()
}
fn default_annotation_font_size() -> f64 {
    32.0
}
fn default_annotation_font_family() -> String {
    "\"SF Pro Display\", \"SF Pro Text\", \"Helvetica Neue\", sans-serif".into()
}
fn default_annotation_radius() -> f64 {
    8.0
}

impl Default for AnnotationTextStyle {
    fn default() -> Self {
        Self {
            color: default_annotation_color(),
            background_color: default_annotation_background(),
            font_size: default_annotation_font_size(),
            font_family: default_annotation_font_family(),
            bold: true,
            border_radius: default_annotation_radius(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FigureData {
    #[serde(default)]
    pub arrow_direction: ArrowDirection,
    #[serde(default = "default_figure_color")]
    pub color: String,
    #[serde(default = "default_figure_stroke")]
    pub stroke_width: f64,
}

fn default_figure_color() -> String {
    "#2563EB".into()
}
fn default_figure_stroke() -> f64 {
    4.0
}

impl Default for FigureData {
    fn default() -> Self {
        Self {
            arrow_direction: ArrowDirection::Right,
            color: default_figure_color(),
            stroke_width: default_figure_stroke(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationRegion {
    pub id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    #[serde(rename = "type", default)]
    pub kind: AnnotationKind,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub position: AnnotationPosition,
    #[serde(default)]
    pub size: AnnotationSize,
    #[serde(default)]
    pub style: AnnotationTextStyle,
    #[serde(default)]
    pub z_index: i32,
    #[serde(default)]
    pub figure: Option<FigureData>,
    #[serde(default)]
    pub blur_intensity: Option<f64>,
}

impl AnnotationRegion {
    pub fn normalized(mut self) -> Self {
        if self.end_ms < self.start_ms {
            std::mem::swap(&mut self.start_ms, &mut self.end_ms);
        }
        self.position.x = clamp_percent(self.position.x);
        self.position.y = clamp_percent(self.position.y);
        self.size.width = clamp_finite_or(self.size.width, 1.0, 100.0, 30.0);
        self.size.height = clamp_finite_or(self.size.height, 1.0, 100.0, 20.0);
        self.style.font_size = clamp_finite_or(self.style.font_size, 8.0, 256.0, 32.0);
        self.style.border_radius = clamp_finite_or(self.style.border_radius, 0.0, 64.0, 8.0);
        if let Some(blur) = self.blur_intensity {
            self.blur_intensity = Some(clamp_finite_or(blur, 0.0, 100.0, 20.0));
        }
        if let Some(mut figure) = self.figure {
            figure.stroke_width = clamp_finite_or(figure.stroke_width, 1.0, 32.0, 4.0);
            self.figure = Some(figure);
        }
        self
    }

    pub fn validate(&self) -> Result<(), RegionOpError> {
        if self.id.trim().is_empty() {
            return Err(RegionOpError::EmptyId);
        }
        validate_range(self.start_ms, self.end_ms)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Audio regions (Phase 2)
// ---------------------------------------------------------------------------

/// Detached audio overlay (music bed, narration, stinger). Mirrors Recordly
/// `AudioRegion`: volume is unit gain clamped to 0..1 on normalize.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioRegion {
    pub id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    #[serde(default)]
    pub audio_path: String,
    #[serde(default = "default_audio_volume")]
    pub volume: f64,
    #[serde(default)]
    pub normalize_audio: bool,
    #[serde(default)]
    pub track_index: Option<u32>,
}

fn default_audio_volume() -> f64 {
    1.0
}

impl AudioRegion {
    pub fn normalized(mut self) -> Self {
        if self.end_ms < self.start_ms {
            std::mem::swap(&mut self.start_ms, &mut self.end_ms);
        }
        self.volume = clamp_finite_or(self.volume, 0.0, 1.0, 1.0);
        self
    }

    pub fn validate(&self) -> Result<(), RegionOpError> {
        if self.id.trim().is_empty() {
            return Err(RegionOpError::EmptyId);
        }
        validate_range(self.start_ms, self.end_ms)?;
        if self.audio_path.trim().is_empty() {
            return Err(RegionOpError::InvalidRange {
                start_ms: self.start_ms,
                end_ms: self.end_ms,
                reason: "audio path must not be empty".into(),
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Undo-safe region ops (Phase 2)
// ---------------------------------------------------------------------------

/// Validation failures for timeline region mutations. Ops check everything
/// before touching the vec, so a failed op leaves state unchanged and the UI
/// must not push an undo entry for it.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RegionOpError {
    #[error("region id must not be empty")]
    EmptyId,
    #[error("region id already exists: {0}")]
    DuplicateId(String),
    #[error("region not found: {0}")]
    NotFound(String),
    #[error("invalid range {start_ms}..{end_ms}: {reason}")]
    InvalidRange {
        start_ms: i64,
        end_ms: i64,
        reason: String,
    },
    #[error("region overlaps an existing region")]
    Overlaps,
    #[error("split point {at_ms} is outside region {start_ms}..{end_ms}")]
    SplitOutside {
        at_ms: i64,
        start_ms: i64,
        end_ms: i64,
    },
}

fn validate_range(start_ms: i64, end_ms: i64) -> Result<(), RegionOpError> {
    if start_ms < 0 || end_ms < 0 {
        return Err(RegionOpError::InvalidRange {
            start_ms,
            end_ms,
            reason: "timestamps must be non-negative".into(),
        });
    }
    if end_ms <= start_ms {
        return Err(RegionOpError::InvalidRange {
            start_ms,
            end_ms,
            reason: "end must be after start".into(),
        });
    }
    Ok(())
}

fn ranges_overlap(a_start: i64, a_end: i64, b_start: i64, b_end: i64) -> bool {
    a_start < b_end && b_start < a_end
}

/// Add a clip after validating id uniqueness, range, and overlap.
/// Pushes the normalized clip on success.
pub fn add_clip_region(
    clips: &mut Vec<ClipRegion>,
    region: ClipRegion,
) -> Result<(), RegionOpError> {
    if region.id.trim().is_empty() {
        return Err(RegionOpError::EmptyId);
    }
    if clips.iter().any(|clip| clip.id == region.id) {
        return Err(RegionOpError::DuplicateId(region.id.clone()));
    }
    validate_range(region.start_ms, region.end_ms)?;
    if clips
        .iter()
        .any(|clip| ranges_overlap(region.start_ms, region.end_ms, clip.start_ms, clip.end_ms))
    {
        return Err(RegionOpError::Overlaps);
    }
    clips.push(region.normalized());
    clips.sort_by_key(|clip| clip.start_ms);
    Ok(())
}

/// Move a clip along the timeline, preserving duration and source in-point
/// (a moved clip keeps reading from the same source offset, mirroring
/// Recordly). Fails without mutating when the target overlaps a sibling.
pub fn move_clip_region(
    clips: &mut [ClipRegion],
    id: &str,
    new_start_ms: i64,
) -> Result<(), RegionOpError> {
    let index = clips
        .iter()
        .position(|clip| clip.id == id)
        .ok_or_else(|| RegionOpError::NotFound(id.into()))?;
    let duration = clips[index].end_ms - clips[index].start_ms;
    if duration <= 0 {
        let clip = &clips[index];
        return Err(RegionOpError::InvalidRange {
            start_ms: clip.start_ms,
            end_ms: clip.end_ms,
            reason: "stored clip has an invalid range".into(),
        });
    }
    if new_start_ms < 0 {
        return Err(RegionOpError::InvalidRange {
            start_ms: new_start_ms,
            end_ms: new_start_ms + duration,
            reason: "timestamps must be non-negative".into(),
        });
    }
    let new_end_ms = new_start_ms + duration;
    if clips.iter().enumerate().any(|(other_index, clip)| {
        other_index != index && ranges_overlap(new_start_ms, new_end_ms, clip.start_ms, clip.end_ms)
    }) {
        return Err(RegionOpError::Overlaps);
    }
    clips[index].start_ms = new_start_ms;
    clips[index].end_ms = new_end_ms;
    clips.sort_by_key(|clip| clip.start_ms);
    Ok(())
}

/// Split a clip at `at_ms` (strictly inside). The left part keeps the
/// original id and source in-point; the right part takes `{id}-b` (or
/// `{id}-b2`, ... on collision) with its source in-point advanced by
/// `(at - start) * speed`, mirroring Recordly's source mapping.
pub fn split_clip_region(
    clips: &mut Vec<ClipRegion>,
    id: &str,
    at_ms: i64,
) -> Result<(), RegionOpError> {
    let index = clips
        .iter()
        .position(|clip| clip.id == id)
        .ok_or_else(|| RegionOpError::NotFound(id.into()))?;
    let clip = clips[index].clone();
    if at_ms <= clip.start_ms || at_ms >= clip.end_ms {
        return Err(RegionOpError::SplitOutside {
            at_ms,
            start_ms: clip.start_ms,
            end_ms: clip.end_ms,
        });
    }
    let speed = if clip.speed.is_finite() && clip.speed > 0.0 {
        clip.speed
    } else {
        1.0
    };
    let right_source =
        clip.source_start() + ((at_ms - clip.start_ms) as f64 * speed).round() as i64;
    let mut suffix = 1;
    let mut right_id = format!("{id}-b");
    while clips.iter().any(|existing| existing.id == right_id) {
        suffix += 1;
        right_id = format!("{id}-b{suffix}");
    }
    let left = ClipRegion {
        id: clip.id.clone(),
        start_ms: clip.start_ms,
        end_ms: at_ms,
        speed: clip.speed,
        muted: clip.muted,
        source_start_ms: clip.source_start_ms,
    };
    let right = ClipRegion {
        id: right_id,
        start_ms: at_ms,
        end_ms: clip.end_ms,
        speed: clip.speed,
        muted: clip.muted,
        source_start_ms: Some(right_source),
    };
    clips[index] = left.normalized();
    clips.push(right.normalized());
    clips.sort_by_key(|clip| clip.start_ms);
    Ok(())
}

/// Trim a clip to a sub-range of itself. Left-trimming advances the source
/// in-point by `(new_start - start) * speed`; right-trimming keeps it.
/// The new range must be contained in the original (trims only shrink).
pub fn trim_clip_region(
    clips: &mut [ClipRegion],
    id: &str,
    new_start_ms: i64,
    new_end_ms: i64,
) -> Result<(), RegionOpError> {
    let index = clips
        .iter()
        .position(|clip| clip.id == id)
        .ok_or_else(|| RegionOpError::NotFound(id.into()))?;
    validate_range(new_start_ms, new_end_ms)?;
    let clip = clips[index].clone();
    if new_start_ms < clip.start_ms || new_end_ms > clip.end_ms {
        return Err(RegionOpError::InvalidRange {
            start_ms: new_start_ms,
            end_ms: new_end_ms,
            reason: "trim range must stay inside the original clip".into(),
        });
    }
    let speed = if clip.speed.is_finite() && clip.speed > 0.0 {
        clip.speed
    } else {
        1.0
    };
    let new_source =
        clip.source_start() + ((new_start_ms - clip.start_ms) as f64 * speed).round() as i64;
    clips[index].start_ms = new_start_ms;
    clips[index].end_ms = new_end_ms;
    clips[index].source_start_ms = Some(new_source.max(0));
    clips.sort_by_key(|clip| clip.start_ms);
    Ok(())
}

/// Add an annotation after id/range validation. Normalizes pos/size/style.
pub fn add_annotation(
    annotations: &mut Vec<AnnotationRegion>,
    region: AnnotationRegion,
) -> Result<(), RegionOpError> {
    region.validate()?;
    if annotations.iter().any(|item| item.id == region.id) {
        return Err(RegionOpError::DuplicateId(region.id.clone()));
    }
    annotations.push(region.normalized());
    annotations.sort_by_key(|item| item.start_ms);
    Ok(())
}

/// Add a detached audio region after id/range/path validation.
pub fn add_audio_region(
    regions: &mut Vec<AudioRegion>,
    region: AudioRegion,
) -> Result<(), RegionOpError> {
    region.validate()?;
    if regions.iter().any(|item| item.id == region.id) {
        return Err(RegionOpError::DuplicateId(region.id.clone()));
    }
    regions.push(region.normalized());
    regions.sort_by_key(|item| item.start_ms);
    Ok(())
}

// ---------------------------------------------------------------------------
// Heuristic zoom suggestions (Phase 3, rule-based only)
// ---------------------------------------------------------------------------

/// One cursor telemetry sample. `x`/`y` are normalized (0..1);
/// `click` marks explicit click events (uiohook telemetry). Rule-based
/// only: no models, no network, no transcription.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorSample {
    pub time_ms: i64,
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub click: bool,
}

/// Suggest zoom regions from cursor telemetry with Recordly's click-cluster
/// heuristic: group explicit clicks separated by at most
/// [`CLICK_CLUSTER_MERGE_GAP_MS`], pad each cluster by
/// [`CLICK_CLUSTER_PAD_MS`], focus on the cluster centroid, depth
/// [`SUGGESTED_ZOOM_DEPTH`], mode [`ZoomMode::Auto`].
///
/// Pure function: same inputs always yield the same suggestions, sorted by
/// start. Returns an empty vec when there is no usable telemetry
/// (`total_ms <= 0`, no clicks, or only out-of-range samples).
pub fn suggest_zoom_regions(samples: &[CursorSample], total_ms: i64) -> Vec<ZoomRegion> {
    if total_ms <= 0 {
        return Vec::new();
    }
    let mut clicks: Vec<(i64, f64, f64)> = samples
        .iter()
        .filter(|sample| sample.click)
        .filter(|sample| sample.time_ms >= 0 && sample.time_ms <= total_ms)
        .filter(|sample| sample.x.is_finite() && sample.y.is_finite())
        .map(|sample| {
            (
                sample.time_ms,
                sample.x.clamp(0.0, 1.0),
                sample.y.clamp(0.0, 1.0),
            )
        })
        .collect();
    if clicks.is_empty() {
        return Vec::new();
    }
    clicks.sort_by_key(|click| click.0);

    // Cluster consecutive clicks separated by at most the merge gap.
    let mut clusters: Vec<Vec<(i64, f64, f64)>> = Vec::new();
    for click in clicks {
        let extend = clusters
            .last()
            .is_some_and(|cluster: &Vec<(i64, f64, f64)>| {
                click.0 - cluster.last().map_or(click.0, |last| last.0)
                    <= CLICK_CLUSTER_MERGE_GAP_MS
            });
        if extend {
            if let Some(cluster) = clusters.last_mut() {
                cluster.push(click);
            }
        } else {
            clusters.push(vec![click]);
        }
    }

    let mut suggestions = Vec::new();
    for (index, cluster) in clusters.iter().enumerate() {
        let first = cluster.first().map_or(0, |click| click.0);
        let last = cluster.last().map_or(0, |click| click.0);
        let start_ms = (first - CLICK_CLUSTER_PAD_MS).max(0);
        let end_ms = (last + CLICK_CLUSTER_PAD_MS).min(total_ms);
        if end_ms <= start_ms {
            continue;
        }
        if suggestions
            .iter()
            .any(|existing: &ZoomRegion| end_ms > existing.start_ms && start_ms < existing.end_ms)
        {
            continue;
        }
        let count = cluster.len() as f64;
        let focus = ZoomFocus {
            cx: cluster.iter().map(|click| click.1).sum::<f64>() / count,
            cy: cluster.iter().map(|click| click.2).sum::<f64>() / count,
        }
        .normalized();
        suggestions.push(
            ZoomRegion {
                id: format!("zoom-suggest-{}", index + 1),
                start_ms,
                end_ms,
                depth: SUGGESTED_ZOOM_DEPTH,
                focus,
                mode: ZoomMode::Auto,
            }
            .normalized(),
        );
    }
    suggestions.sort_by_key(|region| region.start_ms);
    suggestions
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
    pub annotations: Vec<AnnotationRegion>,
    #[serde(default)]
    pub audio_regions: Vec<AudioRegion>,
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
            annotations: vec![],
            audio_regions: vec![],
            webcam: WebcamOverlay::default(),
        }
    }
}

impl EditorState {
    /// Clamp every field into its valid range and drop nothing silently
    /// except inverted ranges, which are repaired by swapping. Total: never
    /// panics, even on NaN/infinite inputs.
    pub fn normalized(mut self) -> Self {
        self.version = EDITOR_SCHEMA_VERSION;
        self.zooms = self.zooms.into_iter().map(ZoomRegion::normalized).collect();
        self.clips = self.clips.into_iter().map(ClipRegion::normalized).collect();
        self.annotations = self
            .annotations
            .into_iter()
            .map(AnnotationRegion::normalized)
            .collect();
        self.audio_regions = self
            .audio_regions
            .into_iter()
            .map(AudioRegion::normalized)
            .collect();
        self.webcam.position_x = clamp01(self.webcam.position_x);
        self.webcam.position_y = clamp01(self.webcam.position_y);
        self.webcam.size = clamp_finite_or(self.webcam.size, 0.05, 1.0, 0.25);
        self.webcam.roundness = clamp_finite_or(self.webcam.roundness, 0.0, 1.0, 0.2);
        self.webcam.shadow = clamp_finite_or(self.webcam.shadow, 0.0, 1.0, 0.4);
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

    #[test]
    fn normalize_is_total_over_nan_inputs() {
        let state = EditorState {
            webcam: WebcamOverlay {
                position_x: f64::NAN,
                position_y: f64::INFINITY,
                size: f64::NAN,
                roundness: f64::NAN,
                shadow: f64::NEG_INFINITY,
                ..WebcamOverlay::default()
            },
            zooms: vec![ZoomRegion {
                id: "z".into(),
                start_ms: 0,
                end_ms: 100,
                depth: 0,
                focus: ZoomFocus {
                    cx: f64::NAN,
                    cy: f64::NAN,
                },
                mode: ZoomMode::Auto,
            }],
            annotations: vec![AnnotationRegion {
                id: "a".into(),
                start_ms: 0,
                end_ms: 100,
                kind: AnnotationKind::Text,
                content: "hi".into(),
                position: AnnotationPosition {
                    x: f64::NAN,
                    y: f64::NAN,
                },
                size: AnnotationSize {
                    width: f64::NAN,
                    height: f64::NAN,
                },
                style: AnnotationTextStyle {
                    font_size: f64::NAN,
                    border_radius: f64::NAN,
                    ..AnnotationTextStyle::default()
                },
                z_index: 0,
                figure: None,
                blur_intensity: Some(f64::NAN),
            }],
            audio_regions: vec![AudioRegion {
                id: "au".into(),
                start_ms: 0,
                end_ms: 100,
                audio_path: "a.wav".into(),
                volume: f64::NAN,
                normalize_audio: false,
                track_index: None,
            }],
            ..EditorState::default()
        }
        .normalized();
        assert!(state.webcam.position_x.is_finite());
        assert!(state.webcam.size.is_finite());
        assert!(state.zooms[0].focus.cx.is_finite());
        assert!(state.annotations[0].position.x.is_finite());
        assert!(state.audio_regions[0].volume.is_finite());
    }

    fn clip(id: &str, start_ms: i64, end_ms: i64) -> ClipRegion {
        ClipRegion {
            id: id.into(),
            start_ms,
            end_ms,
            speed: 1.0,
            muted: false,
            source_start_ms: None,
        }
    }

    #[test]
    fn clip_source_mapping_matches_recordly() {
        let sped = ClipRegion {
            speed: 2.0,
            source_start_ms: Some(1000),
            ..clip("c", 500, 1000)
        };
        assert_eq!(sped.source_start(), 1000);
        assert_eq!(sped.source_end(), 2000);
        assert_eq!(clip("c", 0, 100).source_end(), 100);
    }

    #[test]
    fn add_clip_rejects_duplicates_overlaps_and_bad_ranges() {
        let mut clips = vec![clip("a", 0, 1000)];
        assert_eq!(
            add_clip_region(&mut clips, clip("a", 2000, 3000)),
            Err(RegionOpError::DuplicateId("a".into()))
        );
        assert_eq!(
            add_clip_region(&mut clips, clip("b", 500, 1500)),
            Err(RegionOpError::Overlaps)
        );
        assert!(add_clip_region(&mut clips, clip("", 2000, 3000)).is_err());
        assert!(add_clip_region(&mut clips, clip("c", 900, 900)).is_err());
        // Failed ops leave state untouched (undo-safe).
        assert_eq!(clips, vec![clip("a", 0, 1000)]);
        add_clip_region(&mut clips, clip("b", 1000, 2000)).unwrap();
        assert_eq!(clips.len(), 2);
    }

    #[test]
    fn move_clip_preserves_duration_and_source() {
        let mut clips = vec![
            ClipRegion {
                source_start_ms: Some(5000),
                ..clip("a", 0, 1000)
            },
            clip("b", 2000, 3000),
        ];
        move_clip_region(&mut clips, "a", 3000).unwrap();
        let moved = clips.iter().find(|clip| clip.id == "a").unwrap();
        assert_eq!((moved.start_ms, moved.end_ms), (3000, 4000));
        assert_eq!(moved.source_start_ms, Some(5000));
        // Overlap fails without mutating.
        let before = clips.clone();
        assert_eq!(
            move_clip_region(&mut clips, "a", 2500),
            Err(RegionOpError::Overlaps)
        );
        assert_eq!(clips, before);
        assert!(move_clip_region(&mut clips, "missing", 0).is_err());
    }

    #[test]
    fn split_clip_advances_source_in_point() {
        let mut clips = vec![ClipRegion {
            speed: 2.0,
            source_start_ms: Some(1000),
            ..clip("a", 0, 1000)
        }];
        split_clip_region(&mut clips, "a", 250).unwrap();
        assert_eq!(clips.len(), 2);
        let left = clips.iter().find(|clip| clip.id == "a").unwrap();
        let right = clips.iter().find(|clip| clip.id == "a-b").unwrap();
        assert_eq!((left.start_ms, left.end_ms), (0, 250));
        assert_eq!((right.start_ms, right.end_ms), (250, 1000));
        assert_eq!(right.source_start(), 1500);
        // Boundary splits fail without mutating.
        let before = clips.clone();
        assert!(split_clip_region(&mut clips, "a", 0).is_err());
        assert!(split_clip_region(&mut clips, "a", 250).is_err());
        assert_eq!(clips, before);
    }

    #[test]
    fn trim_clip_shrinks_and_rebases_source() {
        let mut clips = vec![ClipRegion {
            speed: 2.0,
            source_start_ms: Some(1000),
            ..clip("a", 0, 1000)
        }];
        trim_clip_region(&mut clips, "a", 100, 900).unwrap();
        let trimmed = &clips[0];
        assert_eq!((trimmed.start_ms, trimmed.end_ms), (100, 900));
        assert_eq!(trimmed.source_start(), 1200);
        // Growing beyond the original fails without mutating.
        let before = clips.clone();
        assert!(trim_clip_region(&mut clips, "a", 0, 2000).is_err());
        assert_eq!(clips, before);
    }

    #[test]
    fn annotation_and_audio_regions_normalize_and_validate() {
        let annotation = AnnotationRegion {
            id: "n1".into(),
            start_ms: 500,
            end_ms: 100,
            kind: AnnotationKind::Blur,
            content: String::new(),
            position: AnnotationPosition { x: 999.0, y: -5.0 },
            size: AnnotationSize {
                width: 0.0,
                height: 500.0,
            },
            style: AnnotationTextStyle::default(),
            z_index: 1,
            figure: Some(FigureData {
                stroke_width: 99.0,
                ..FigureData::default()
            }),
            blur_intensity: Some(500.0),
        }
        .normalized();
        assert_eq!((annotation.start_ms, annotation.end_ms), (100, 500));
        assert_eq!((annotation.position.x, annotation.position.y), (100.0, 0.0));

        let mut annotations = Vec::new();
        add_annotation(&mut annotations, annotation.clone()).unwrap();
        assert_eq!(
            add_annotation(&mut annotations, annotation),
            Err(RegionOpError::DuplicateId("n1".into()))
        );

        let audio = AudioRegion {
            id: "au1".into(),
            start_ms: 0,
            end_ms: 1000,
            audio_path: "media/music.wav".into(),
            volume: 9.0,
            normalize_audio: true,
            track_index: Some(0),
        }
        .normalized();
        assert_eq!(audio.volume, 1.0);
        let mut regions = Vec::new();
        add_audio_region(&mut regions, audio).unwrap();
        assert!(
            add_audio_region(
                &mut regions,
                AudioRegion {
                    id: "bad".into(),
                    start_ms: 0,
                    end_ms: 10,
                    audio_path: String::new(),
                    volume: 1.0,
                    normalize_audio: false,
                    track_index: None,
                },
            )
            .is_err()
        );
    }

    #[test]
    fn zoom_suggestions_cluster_clicks_like_recordly() {
        let samples = vec![
            CursorSample {
                time_ms: 5_000,
                x: 0.5,
                y: 0.5,
                click: true,
            },
            CursorSample {
                time_ms: 0,
                x: 0.1,
                y: 0.1,
                click: false,
            },
        ];
        let suggestions = suggest_zoom_regions(&samples, 30_000);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(
            (suggestions[0].start_ms, suggestions[0].end_ms),
            (5_000 - CLICK_CLUSTER_PAD_MS, 5_000 + CLICK_CLUSTER_PAD_MS)
        );
        assert_eq!(suggestions[0].depth, SUGGESTED_ZOOM_DEPTH);
        assert_eq!(suggestions[0].mode, ZoomMode::Auto);

        // Nearby clicks merge; distant clicks split.
        let clustered = vec![
            CursorSample {
                time_ms: 1_000,
                x: 0.2,
                y: 0.2,
                click: true,
            },
            CursorSample {
                time_ms: 2_000,
                x: 0.3,
                y: 0.3,
                click: true,
            },
            CursorSample {
                time_ms: 20_000,
                x: 0.8,
                y: 0.8,
                click: true,
            },
        ];
        let suggestions = suggest_zoom_regions(&clustered, 30_000);
        assert_eq!(suggestions.len(), 2);
        assert_eq!(
            (suggestions[0].start_ms, suggestions[0].end_ms),
            (500, 2500)
        );
        assert_eq!(
            (suggestions[1].start_ms, suggestions[1].end_ms),
            (19_500, 20_500)
        );
    }

    #[test]
    fn zoom_suggestions_ignore_unusable_telemetry() {
        assert!(suggest_zoom_regions(&[], 30_000).is_empty());
        assert!(
            suggest_zoom_regions(
                &[CursorSample {
                    time_ms: 1_000,
                    x: 0.5,
                    y: 0.5,
                    click: true,
                }],
                0,
            )
            .is_empty()
        );
        // Moves without clicks -> no suggestions (heuristic only, no models).
        assert!(
            suggest_zoom_regions(
                &[CursorSample {
                    time_ms: 1_000,
                    x: 0.5,
                    y: 0.5,
                    click: false,
                }],
                30_000,
            )
            .is_empty()
        );
        // Non-finite positions are dropped, valid clicks still suggest.
        let mixed = vec![
            CursorSample {
                time_ms: 1_000,
                x: f64::NAN,
                y: 0.5,
                click: true,
            },
            CursorSample {
                time_ms: 2_000,
                x: 0.4,
                y: 0.4,
                click: true,
            },
        ];
        assert_eq!(suggest_zoom_regions(&mixed, 30_000).len(), 1);
    }
}
