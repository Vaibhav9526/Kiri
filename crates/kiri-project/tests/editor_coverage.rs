use kiri_project::editor::{
    AnnotationRegion, Appearance, AudioRegion, ClipRegion, CropRegion, EDITOR_SCHEMA_VERSION,
    EditorState, WebcamOverlay, ZoomFocus, ZoomMode, ZoomRegion,
};

fn zoom(id: &str) -> ZoomRegion {
    ZoomRegion {
        id: id.into(),
        start_ms: 0,
        end_ms: 500,
        depth: 2,
        focus: ZoomFocus { cx: 0.5, cy: 0.5 },
        mode: ZoomMode::Manual,
    }
}

fn clip(id: &str) -> ClipRegion {
    ClipRegion {
        id: id.into(),
        start_ms: 0,
        end_ms: 1000,
        speed: 1.0,
        muted: false,
        source_start_ms: None,
    }
}

#[test]
fn zoom_normalization_repairs_ranges_depth_and_focus() {
    let repaired = ZoomRegion {
        start_ms: 900,
        end_ms: 100,
        depth: 99,
        focus: ZoomFocus { cx: 9.0, cy: -2.0 },
        ..zoom("z1")
    }
    .normalized();
    assert_eq!((repaired.start_ms, repaired.end_ms), (100, 900));
    assert_eq!(repaired.depth, 6);
    assert_eq!(repaired.focus.cx, 1.0);
    assert_eq!(repaired.focus.cy, 0.0);

    let floored = ZoomRegion {
        depth: 0,
        ..zoom("z2")
    }
    .normalized();
    assert_eq!(floored.depth, 1);
}

#[test]
fn clip_normalization_repairs_ranges_and_speeds() {
    let swapped = ClipRegion {
        start_ms: 800,
        end_ms: 200,
        ..clip("c1")
    }
    .normalized();
    assert_eq!((swapped.start_ms, swapped.end_ms), (200, 800));

    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -2.0] {
        let fixed = ClipRegion {
            speed: bad,
            ..clip("c2")
        }
        .normalized();
        assert_eq!(fixed.speed, 1.0, "speed {bad} should reset to 1.0");
    }

    assert_eq!(
        ClipRegion {
            speed: 0.1,
            ..clip("c3")
        }
        .normalized()
        .speed,
        0.25
    );
    assert_eq!(
        ClipRegion {
            speed: 8.0,
            ..clip("c4")
        }
        .normalized()
        .speed,
        4.0
    );
    // Negative source in-points are dropped back to timeline-relative reads.
    let negative = ClipRegion {
        source_start_ms: Some(-50),
        ..clip("c5")
    }
    .normalized();
    assert_eq!(negative.source_start_ms, None);
    assert_eq!(negative.source_start(), negative.start_ms);
}

#[test]
fn clip_source_math_follows_display_duration_and_speed() {
    let unit = clip("c1");
    assert_eq!(unit.source_start(), 0);
    assert_eq!(unit.source_end(), 1000);

    let doubled = ClipRegion {
        speed: 2.0,
        ..clip("c2")
    };
    assert_eq!(doubled.source_end(), 2000);

    let rebased = ClipRegion {
        source_start_ms: Some(400),
        ..clip("c3")
    };
    assert_eq!(rebased.source_start(), 400);
    assert_eq!(rebased.source_end(), 1400);
}

#[test]
fn editor_state_normalization_versions_and_clamps_webcam() {
    let state = EditorState {
        version: 0,
        webcam: WebcamOverlay {
            position_x: 9.0,
            position_y: -4.0,
            size: 99.0,
            roundness: -1.0,
            shadow: 42.0,
            ..WebcamOverlay::default()
        },
        zooms: vec![ZoomRegion {
            start_ms: 5,
            end_ms: 1,
            ..zoom("z1")
        }],
        ..EditorState::default()
    };
    let normalized = state.normalized();
    assert_eq!(normalized.version, EDITOR_SCHEMA_VERSION);
    assert_eq!(normalized.webcam.position_x, 1.0);
    assert_eq!(normalized.webcam.position_y, 0.0);
    assert_eq!(normalized.webcam.size, 1.0);
    assert_eq!(normalized.webcam.roundness, 0.0);
    assert_eq!(normalized.webcam.shadow, 1.0);
    assert_eq!(
        (normalized.zooms[0].start_ms, normalized.zooms[0].end_ms),
        (1, 5)
    );
}

#[test]
fn editor_state_normalization_is_total_over_nan_inputs() {
    let state = EditorState {
        webcam: WebcamOverlay {
            size: f64::NAN,
            roundness: f64::NAN,
            shadow: f64::INFINITY,
            ..WebcamOverlay::default()
        },
        ..EditorState::default()
    };
    let normalized = state.normalized();
    assert!(normalized.webcam.size.is_finite());
    assert!(normalized.webcam.roundness.is_finite());
    assert!(normalized.webcam.shadow.is_finite());
}

#[test]
fn editor_defaults_are_sane_and_round_trip() {
    let state = EditorState::default();
    assert_eq!(state.version, EDITOR_SCHEMA_VERSION);
    assert!(state.webcam.mirror);
    assert_eq!(state.appearance, Appearance::default());
    assert_eq!(state.webcam, WebcamOverlay::default());
    assert_eq!(CropRegion::default().width, 1.0);
    let json = serde_json::to_string(&state).unwrap();
    let decoded: EditorState = serde_json::from_str(&json).unwrap();
    assert_eq!(state, decoded);
}

#[test]
fn editor_deserialization_applies_zoom_and_caption_defaults() {
    let zoom_value = serde_json::json!({
        "id": "z1",
        "startMs": 0,
        "endMs": 500,
        "depth": 2,
        "focus": { "cx": 0.5, "cy": 0.5 },
    });
    let parsed: ZoomRegion = serde_json::from_value(zoom_value).unwrap();
    assert_eq!(parsed.mode, ZoomMode::Manual);

    let caption_value = serde_json::json!({
        "id": "cap1",
        "startMs": 0,
        "endMs": 500,
        "text": "Hello",
    });
    let caption: kiri_project::editor::CaptionCue = serde_json::from_value(caption_value).unwrap();
    assert!(caption.words.is_empty());
}

#[test]
fn annotation_and_audio_regions_normalize_and_validate() {
    let annotation = AnnotationRegion {
        id: "a1".into(),
        start_ms: 300,
        end_ms: 100,
        kind: Default::default(),
        content: String::new(),
        position: kiri_project::editor::AnnotationPosition { x: 500.0, y: -20.0 },
        size: kiri_project::editor::AnnotationSize {
            width: 0.0,
            height: 500.0,
        },
        style: Default::default(),
        z_index: 0,
        figure: None,
        blur_intensity: Some(500.0),
    }
    .normalized();
    assert_eq!((annotation.start_ms, annotation.end_ms), (100, 300));
    assert_eq!(annotation.position.x, 100.0);
    assert_eq!(annotation.position.y, 0.0);
    assert_eq!(annotation.size.width, 1.0);
    assert_eq!(annotation.size.height, 100.0);
    assert_eq!(annotation.blur_intensity, Some(100.0));
    assert!(annotation.validate().is_ok());
    assert!(
        AnnotationRegion {
            id: "   ".into(),
            ..annotation.clone()
        }
        .validate()
        .is_err()
    );

    let audio = AudioRegion {
        id: "au1".into(),
        start_ms: 0,
        end_ms: 1000,
        audio_path: "media/music.wav".into(),
        volume: 4.0,
        normalize_audio: false,
        track_index: None,
    }
    .normalized();
    assert_eq!(audio.volume, 1.0);
    assert!(audio.validate().is_ok());
    assert!(
        AudioRegion {
            audio_path: String::new(),
            ..audio.clone()
        }
        .validate()
        .is_err()
    );
}
