//! Phase 2 hardening: region ops are undo-safe (validate-before-mutate).
//!
//! Every failing op must leave state untouched so the UI only pushes undo
//! entries for successful ops. Covers trim/split/delete, zoom regions,
//! camera/background normalization, and overflow saturation.

use kiri_project::editor::{
    AnnotationKind, AnnotationPosition, AnnotationRegion, AnnotationSize, AnnotationTextStyle,
    AudioRegion, CaptionCue, CaptionWord, ClipRegion, EditorState, RegionOpError, SpeedRegion,
    TrimRegion, ZoomFocus, ZoomMode, ZoomRegion, add_annotation, add_audio_region, add_caption,
    add_clip_region, add_speed_region, add_trim_region, add_zoom_region, delete_annotation,
    delete_audio_region, delete_caption, delete_clip_region, delete_speed_region,
    delete_trim_region, delete_zoom_region, move_clip_region, split_clip_region, trim_clip_region,
};

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

fn zoom(id: &str, start_ms: i64, end_ms: i64) -> ZoomRegion {
    ZoomRegion {
        id: id.into(),
        start_ms,
        end_ms,
        depth: 2,
        focus: ZoomFocus { cx: 0.5, cy: 0.5 },
        mode: ZoomMode::Manual,
    }
}

fn assert_untouched<T: Clone + PartialEq + std::fmt::Debug>(before: &[T], after: &[T], op: &str) {
    assert_eq!(before, after, "failed op mutated state: {op}");
}

#[test]
fn clip_add_move_split_trim_delete_are_undo_safe() {
    let mut clips = vec![clip("a", 0, 1000)];

    // Add failures leave state untouched.
    let before = clips.clone();
    assert_eq!(
        add_clip_region(&mut clips, clip("a", 2000, 3000)),
        Err(RegionOpError::DuplicateId("a".into()))
    );
    assert_untouched(&before, &clips, "add duplicate");
    assert!(add_clip_region(&mut clips, clip("b", 500, 1500)).is_err());
    assert_untouched(&before, &clips, "add overlap");
    assert!(add_clip_region(&mut clips, clip("", 2000, 3000)).is_err());
    assert_untouched(&before, &clips, "add empty id");
    assert!(add_clip_region(&mut clips, clip("z", 5, 5)).is_err());
    assert_untouched(&before, &clips, "add zero range");

    // Move failure (overlap / missing / negative) leaves state untouched.
    add_clip_region(&mut clips, clip("b", 1000, 2000)).unwrap();
    let before = clips.clone();
    assert_eq!(
        move_clip_region(&mut clips, "a", 1500),
        Err(RegionOpError::Overlaps)
    );
    assert_untouched(&before, &clips, "move overlap");
    assert!(move_clip_region(&mut clips, "missing", 0).is_err());
    assert_untouched(&before, &clips, "move missing");
    assert!(move_clip_region(&mut clips, "a", -5).is_err());
    assert_untouched(&before, &clips, "move negative");

    // Split failures (boundary / missing) leave state untouched.
    let before = clips.clone();
    assert!(split_clip_region(&mut clips, "a", 0).is_err());
    assert!(split_clip_region(&mut clips, "a", 1000).is_err());
    assert!(split_clip_region(&mut clips, "missing", 500).is_err());
    assert_untouched(&before, &clips, "split boundary");

    // Split success advances the source in-point for sped-up clips.
    let mut sped = vec![ClipRegion {
        speed: 2.0,
        source_start_ms: Some(1000),
        ..clip("s", 0, 1000)
    }];
    split_clip_region(&mut sped, "s", 250).unwrap();
    let right = sped.iter().find(|c| c.id == "s-b").unwrap();
    assert_eq!(right.source_start(), 1500);

    // Split id collision resolves without duplicating (`-b2`).
    let mut collision = vec![clip("k", 0, 1000), clip("k-b", 2000, 3000)];
    split_clip_region(&mut collision, "k", 400).unwrap();
    assert!(collision.iter().any(|c| c.id == "k-b2"));

    // Trim failures (grow / inverted / missing) leave state untouched.
    let before = clips.clone();
    assert!(trim_clip_region(&mut clips, "a", 0, 5000).is_err());
    assert_untouched(&before, &clips, "trim grow");
    assert!(trim_clip_region(&mut clips, "a", 900, 100).is_err());
    assert_untouched(&before, &clips, "trim inverted");
    assert!(trim_clip_region(&mut clips, "missing", 0, 10).is_err());
    assert_untouched(&before, &clips, "trim missing");

    // Trim success rebases the source in-point.
    let mut trimmable = vec![ClipRegion {
        speed: 2.0,
        source_start_ms: Some(1000),
        ..clip("t", 0, 1000)
    }];
    trim_clip_region(&mut trimmable, "t", 100, 900).unwrap();
    assert_eq!(trimmable[0].source_start(), 1200);

    // Delete missing leaves state untouched; delete success returns the clip.
    let before = clips.clone();
    assert_eq!(
        delete_clip_region(&mut clips, "missing"),
        Err(RegionOpError::NotFound("missing".into()))
    );
    assert_untouched(&before, &clips, "delete missing");
    let removed = delete_clip_region(&mut clips, "a").unwrap();
    assert_eq!(removed.id, "a");
    assert!(clips.iter().all(|c| c.id != "a"));
}

#[test]
fn zoom_trim_speed_caption_ops_are_undo_safe() {
    let mut zooms = vec![zoom("z1", 0, 500)];
    let before = zooms.clone();
    assert!(add_zoom_region(&mut zooms, zoom("z1", 600, 900)).is_err());
    assert_untouched(&before, &zooms, "zoom duplicate");
    assert!(add_zoom_region(&mut zooms, zoom("", 600, 900)).is_err());
    assert_untouched(&before, &zooms, "zoom empty id");
    assert!(add_zoom_region(&mut zooms, zoom("bad", 9, 9)).is_err());
    assert_untouched(&before, &zooms, "zoom zero range");
    // Zooms may overlap: this is allowed and sorted.
    add_zoom_region(&mut zooms, zoom("z2", 250, 750)).unwrap();
    assert_eq!(zooms.len(), 2);
    assert!(zooms[0].start_ms <= zooms[1].start_ms);
    assert_eq!(
        delete_zoom_region(&mut zooms, "missing"),
        Err(RegionOpError::NotFound("missing".into()))
    );
    assert_eq!(zooms.len(), 2);
    assert_eq!(delete_zoom_region(&mut zooms, "z1").unwrap().id, "z1");

    let mut trims = Vec::new();
    add_trim_region(
        &mut trims,
        TrimRegion {
            id: "t1".into(),
            start_ms: 0,
            end_ms: 100,
        },
    )
    .unwrap();
    let before = trims.clone();
    assert!(
        add_trim_region(
            &mut trims,
            TrimRegion {
                id: "t1".into(),
                start_ms: 200,
                end_ms: 300
            }
        )
        .is_err()
    );
    assert_untouched(&before, &trims, "trim duplicate");
    assert_eq!(
        delete_trim_region(&mut trims, "missing"),
        Err(RegionOpError::NotFound("missing".into()))
    );
    assert_untouched(&before, &trims, "trim delete missing");

    let mut speeds = Vec::new();
    add_speed_region(
        &mut speeds,
        SpeedRegion {
            id: "s1".into(),
            start_ms: 0,
            end_ms: 100,
            speed: 1.5,
        },
    )
    .unwrap();
    let before = speeds.clone();
    assert!(
        add_speed_region(
            &mut speeds,
            SpeedRegion {
                id: "s1".into(),
                start_ms: 200,
                end_ms: 300,
                speed: 1.5
            }
        )
        .is_err()
    );
    assert_untouched(&before, &speeds, "speed duplicate");
    assert!(
        add_speed_region(
            &mut speeds,
            SpeedRegion {
                id: "bad".into(),
                start_ms: 0,
                end_ms: 10,
                speed: f64::NAN
            }
        )
        .is_err()
    );
    assert_untouched(&before, &speeds, "speed NaN");
    assert_eq!(
        delete_speed_region(&mut speeds, "missing"),
        Err(RegionOpError::NotFound("missing".into()))
    );
    assert_untouched(&before, &speeds, "speed delete missing");

    let mut captions = Vec::new();
    add_caption(
        &mut captions,
        CaptionCue {
            id: "c1".into(),
            start_ms: 0,
            end_ms: 100,
            text: "hi".into(),
            words: vec![],
        },
    )
    .unwrap();
    let before = captions.clone();
    assert!(
        add_caption(
            &mut captions,
            CaptionCue {
                id: "c1".into(),
                start_ms: 200,
                end_ms: 300,
                text: "x".into(),
                words: vec![]
            }
        )
        .is_err()
    );
    assert_untouched(&before, &captions, "caption duplicate");
    assert_eq!(
        delete_caption(&mut captions, "missing"),
        Err(RegionOpError::NotFound("missing".into()))
    );
    assert_untouched(&before, &captions, "caption delete missing");
}

#[test]
fn annotation_audio_delete_is_undo_safe() {
    let mut annotations = Vec::new();
    let region = AnnotationRegion {
        id: "n1".into(),
        start_ms: 0,
        end_ms: 100,
        kind: AnnotationKind::Text,
        content: "hi".into(),
        position: AnnotationPosition { x: 50.0, y: 50.0 },
        size: AnnotationSize {
            width: 30.0,
            height: 20.0,
        },
        style: AnnotationTextStyle::default(),
        z_index: 0,
        figure: None,
        blur_intensity: None,
    };
    add_annotation(&mut annotations, region.clone()).unwrap();
    let before = annotations.clone();
    assert_eq!(
        add_annotation(&mut annotations, region),
        Err(RegionOpError::DuplicateId("n1".into()))
    );
    assert_untouched(&before, &annotations, "annotation duplicate");
    assert_eq!(
        delete_annotation(&mut annotations, "missing"),
        Err(RegionOpError::NotFound("missing".into()))
    );
    assert_untouched(&before, &annotations, "annotation delete missing");

    let mut audio = Vec::new();
    add_audio_region(
        &mut audio,
        AudioRegion {
            id: "au1".into(),
            start_ms: 0,
            end_ms: 100,
            audio_path: "media/a.wav".into(),
            volume: 1.0,
            normalize_audio: false,
            track_index: None,
        },
    )
    .unwrap();
    let before = audio.clone();
    assert!(
        add_audio_region(
            &mut audio,
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
    assert_untouched(&before, &audio, "audio empty path");
    assert_eq!(
        delete_audio_region(&mut audio, "missing"),
        Err(RegionOpError::NotFound("missing".into()))
    );
    assert_untouched(&before, &audio, "audio delete missing");
}

#[test]
fn region_ops_saturate_on_extreme_timelines() {
    // None of these may panic; overflow resolves via saturation + clean errors.
    let mut clips = vec![clip("edge", 0, 1000)];
    let _ = move_clip_region(&mut clips, "edge", i64::MAX);
    let _ = split_clip_region(&mut clips, "edge", i64::MAX);
    let _ = trim_clip_region(&mut clips, "edge", 0, i64::MAX);

    let extreme = ClipRegion {
        source_start_ms: Some(i64::MAX),
        speed: 4.0,
        ..clip("x", i64::MIN, i64::MAX)
    };
    let _ = extreme.source_start();
    let _ = extreme.source_end();

    let samples = vec![
        kiri_project::editor::CursorSample {
            time_ms: i64::MAX,
            x: 0.5,
            y: 0.5,
            click: true,
        },
        kiri_project::editor::CursorSample {
            time_ms: i64::MIN,
            x: 0.5,
            y: 0.5,
            click: true,
        },
    ];
    let _ = kiri_project::editor::suggest_zoom_regions(&samples, i64::MAX);
}

#[test]
fn camera_overlay_and_backgrounds_normalize_totally() {
    // Camera overlay extremes clamp; backgrounds reset invalid values.
    let state = EditorState {
        appearance: kiri_project::editor::Appearance {
            background: String::new(),
            padding: f64::NAN,
            border_radius: f64::INFINITY,
            shadow: f64::NEG_INFINITY,
            aspect_ratio: Some("not-a-ratio".into()),
        },
        webcam: kiri_project::editor::WebcamOverlay {
            position_x: f64::NAN,
            position_y: f64::INFINITY,
            size: -99.0,
            roundness: 99.0,
            shadow: -5.0,
            crop: kiri_project::editor::CropRegion {
                x: 9.0,
                y: 9.0,
                width: 9.0,
                height: 9.0,
            },
            source_path: Some("   ".into()),
            ..kiri_project::editor::WebcamOverlay::default()
        },
        ..EditorState::default()
    }
    .normalized();
    assert!(!state.appearance.background.is_empty());
    assert!(state.appearance.padding.is_finite());
    assert!(state.appearance.aspect_ratio.is_none());
    assert!(state.webcam.position_x.is_finite());
    assert!((0.05..=1.0).contains(&state.webcam.size));
    assert!((0.0..=1.0).contains(&state.webcam.roundness));
    assert!(state.webcam.source_path.is_none());
    assert_eq!(state.clone().normalized(), state);

    // Words clamp inside their cue; empty cue text is kept (delete is explicit).
    let cue = CaptionCue {
        id: "w".into(),
        start_ms: 100,
        end_ms: 200,
        text: String::new(),
        words: vec![CaptionWord {
            text: "x".into(),
            start_ms: 0,
            end_ms: 99_999,
        }],
    }
    .normalized();
    assert!(cue.words[0].start_ms >= 100);
    assert!(cue.words[0].end_ms <= 200);
}
