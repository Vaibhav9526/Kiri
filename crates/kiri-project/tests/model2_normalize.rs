//! Phase 2 hardening: normalize totality + idempotence.
//!
//! Every `normalized()` must be total (never panic on NaN/inf/negative/
//! inverted/extreme inputs) and idempotent
//! (`normalize(normalize(x)) == normalize(x)`).

use kiri_project::editor::{
    AnnotationKind, AnnotationPosition, AnnotationRegion, AnnotationSize, AnnotationTextStyle,
    Appearance, AudioRegion, CaptionCue, CaptionWord, ClipRegion, CropRegion, EditorState,
    SpeedRegion, TrimRegion, WebcamOverlay, ZoomFocus, ZoomMode, ZoomRegion,
};

fn zoom(id: &str, start_ms: i64, end_ms: i64, depth: u32, cx: f64, cy: f64) -> ZoomRegion {
    ZoomRegion {
        id: id.into(),
        start_ms,
        end_ms,
        depth,
        focus: ZoomFocus { cx, cy },
        mode: ZoomMode::Manual,
    }
}

fn clip(id: &str, start_ms: i64, end_ms: i64, speed: f64) -> ClipRegion {
    ClipRegion {
        id: id.into(),
        start_ms,
        end_ms,
        speed,
        muted: false,
        source_start_ms: None,
    }
}

#[test]
fn zoom_normalize_is_idempotent_over_hostile_inputs() {
    let hostile_depths = [0, 1, 6, 7, 99, u32::MAX];
    let hostile_times: &[(i64, i64)] = &[
        (900, 100),
        (100, 100),
        (0, 0),
        (-50, 100),
        (100, -50),
        (i64::MAX, i64::MAX),
        (i64::MIN, i64::MAX),
        (i64::MAX, i64::MIN),
        (0, i64::MAX),
    ];
    let hostile_floats = [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -1.0,
        0.0,
        0.5,
        1.0,
        2.0,
        9.0,
    ];
    for (ti, (start, end)) in hostile_times.iter().enumerate() {
        for depth in hostile_depths {
            for cx in hostile_floats {
                for cy in hostile_floats {
                    let once = zoom(&format!("z{ti}"), *start, *end, depth, cx, cy).normalized();
                    let twice = once.clone().normalized();
                    assert_eq!(once, twice, "zoom not idempotent for {start}..{end}");
                    assert!(once.start_ms <= once.end_ms || once.start_ms == i64::MAX);
                    assert!((1..=6).contains(&once.depth));
                    assert!(once.focus.cx.is_finite());
                    assert!(once.focus.cy.is_finite());
                    assert!((0.0..=1.0).contains(&once.focus.cx));
                    assert!((0.0..=1.0).contains(&once.focus.cy));
                }
            }
        }
    }
}

#[test]
fn clip_normalize_is_idempotent_and_never_panics() {
    let speeds = [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -2.0,
        0.0,
        0.1,
        0.25,
        1.0,
        4.0,
        8.0,
        1e300,
    ];
    let ranges: &[(i64, i64)] = &[
        (800, 200),
        (0, 0),
        (-10, 10),
        (i64::MAX - 1, i64::MAX),
        (i64::MIN, i64::MIN + 1),
        (i64::MIN, i64::MAX),
    ];
    for (i, (start, end)) in ranges.iter().enumerate() {
        for speed in speeds {
            for source in [None, Some(-50), Some(0), Some(400), Some(i64::MAX)] {
                let region = ClipRegion {
                    source_start_ms: source,
                    ..clip(&format!("c{i}"), *start, *end, speed)
                };
                let once = region.normalized();
                let twice = once.clone().normalized();
                assert_eq!(once, twice);
                assert!(once.speed.is_finite());
                assert!((0.25..=4.0).contains(&once.speed));
                // source_start()/source_end() must never panic, even saturated.
                let _ = once.source_start();
                let _ = once.source_end();
            }
        }
    }
}

#[test]
fn source_end_saturates_instead_of_panicking() {
    let extreme = ClipRegion {
        source_start_ms: Some(i64::MAX),
        ..clip("big", 0, i64::MAX, 4.0)
    }
    .normalized();
    let end = extreme.source_end();
    assert_eq!(end, i64::MAX);

    let inverted_extreme = clip("inv", i64::MAX, i64::MIN, 2.0).normalized();
    let _ = inverted_extreme.source_end();

    let neg_speed = clip("ns", 0, 1000, f64::NAN).normalized();
    assert_eq!(neg_speed.source_end(), 1000);
}

#[test]
fn trim_speed_caption_normalize_is_idempotent() {
    for (start, end) in [(300, 100), (50, 50), (-5, 10), (i64::MAX, i64::MAX)] {
        let trim = TrimRegion {
            id: "t".into(),
            start_ms: start,
            end_ms: end,
        }
        .normalized();
        assert_eq!(trim.clone().normalized(), trim);
        assert!(trim.end_ms > trim.start_ms || trim.start_ms == i64::MAX);

        for speed in [f64::NAN, -1.0, 0.0, 0.3, 1.0, 1.6, 9.0, f64::INFINITY] {
            let region = SpeedRegion {
                id: "s".into(),
                start_ms: start,
                end_ms: end,
                speed,
            }
            .normalized();
            assert_eq!(region.clone().normalized(), region);
            assert!(region.speed.is_finite());
        }

        let cue = CaptionCue {
            id: "cap".into(),
            start_ms: start,
            end_ms: end,
            text: "hi".into(),
            words: vec![
                CaptionWord {
                    text: "hi".into(),
                    start_ms: end,
                    end_ms: start,
                },
                CaptionWord {
                    text: "yo".into(),
                    start_ms: i64::MIN,
                    end_ms: i64::MAX,
                },
            ],
        }
        .normalized();
        assert_eq!(cue.clone().normalized(), cue);
        assert!(cue.end_ms > cue.start_ms || cue.start_ms == i64::MAX);
        for word in &cue.words {
            assert!(word.start_ms >= cue.start_ms);
            assert!(word.end_ms <= cue.end_ms);
        }
    }
}

#[test]
fn crop_webcam_appearance_normalize_is_idempotent() {
    let hostile = [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -9.0,
        0.0,
        0.5,
        1.0,
        9.0,
    ];
    for x in hostile {
        for y in hostile {
            for w in hostile {
                for h in hostile {
                    let crop = CropRegion {
                        x,
                        y,
                        width: w,
                        height: h,
                    }
                    .normalized();
                    let twice = crop.clone().normalized();
                    assert_eq!(crop, twice, "crop not idempotent for {x},{y},{w},{h}");
                    assert!(crop.x.is_finite() && crop.y.is_finite());
                    assert!(crop.width.is_finite() && crop.height.is_finite());
                    assert!((0.0..=1.0).contains(&crop.x));
                    assert!((0.0..=1.0).contains(&crop.y));
                    assert!(crop.width >= 0.01 && crop.width <= 1.0);
                    assert!(crop.height >= 0.01 && crop.height <= 1.0);
                    assert!(crop.x + crop.width <= 1.0 + f64::EPSILON);
                    assert!(crop.y + crop.height <= 1.0 + f64::EPSILON);
                }
            }
        }
    }

    for background in ["", "   ", "#0f1115", "wallpaper.jpg"] {
        for padding in [f64::NAN, -5.0, 48.0, 999.0, f64::INFINITY] {
            let appearance = Appearance {
                background: background.into(),
                padding,
                border_radius: padding,
                shadow: padding,
                aspect_ratio: Some("BOGUS".into()),
            }
            .normalized();
            assert_eq!(appearance.clone().normalized(), appearance);
            assert!(!appearance.background.trim().is_empty());
            assert!(appearance.padding.is_finite());
            assert!(appearance.border_radius.is_finite());
            assert!(appearance.shadow.is_finite());
            assert!(appearance.aspect_ratio.is_none());
        }
    }
    let valid_ratio = Appearance {
        aspect_ratio: Some("16:9".into()),
        ..Appearance::default()
    }
    .normalized();
    assert_eq!(valid_ratio.aspect_ratio, Some("16:9".into()));

    let webcam = WebcamOverlay {
        position_x: f64::NAN,
        position_y: f64::INFINITY,
        size: f64::NAN,
        roundness: f64::NEG_INFINITY,
        shadow: f64::NAN,
        crop: CropRegion {
            x: 9.0,
            y: -9.0,
            width: f64::NAN,
            height: f64::INFINITY,
        },
        source_path: Some("".into()),
        ..WebcamOverlay::default()
    }
    .normalized();
    assert_eq!(webcam.clone().normalized(), webcam);
    assert!(webcam.position_x.is_finite());
    assert!(webcam.size.is_finite());
    assert!(webcam.crop.width.is_finite());
    assert!(webcam.source_path.is_none());
}

#[test]
fn annotation_audio_normalize_is_idempotent() {
    let annotation = AnnotationRegion {
        id: "n".into(),
        start_ms: 500,
        end_ms: 100,
        kind: AnnotationKind::Blur,
        content: String::new(),
        position: AnnotationPosition { x: 999.0, y: -5.0 },
        size: AnnotationSize {
            width: 0.0,
            height: 500.0,
        },
        style: AnnotationTextStyle {
            font_size: f64::NAN,
            border_radius: f64::INFINITY,
            ..AnnotationTextStyle::default()
        },
        z_index: 1,
        figure: Some(kiri_project::editor::FigureData {
            stroke_width: f64::NAN,
            ..Default::default()
        }),
        blur_intensity: Some(f64::NAN),
    }
    .normalized();
    assert_eq!(annotation.clone().normalized(), annotation);
    assert!(annotation.position.x.is_finite());
    assert!(annotation.style.font_size.is_finite());

    for volume in [f64::NAN, f64::INFINITY, -1.0, 0.5, 9.0] {
        let audio = AudioRegion {
            id: "au".into(),
            start_ms: 300,
            end_ms: 100,
            audio_path: "media/music.wav".into(),
            volume,
            normalize_audio: false,
            track_index: None,
        }
        .normalized();
        assert_eq!(audio.clone().normalized(), audio);
        assert!(audio.volume.is_finite());
        assert!((0.0..=1.0).contains(&audio.volume));
    }
}

#[test]
fn editor_state_normalize_is_idempotent_over_garbage() {
    // Deterministic PRNG (xorshift) so the property test needs no new deps.
    let mut rng = 0x9E3779B97F4A7C15u64;
    let mut next_u64 = move || {
        rng ^= rng >> 12;
        rng ^= rng << 25;
        rng ^= rng >> 27;
        rng = rng.wrapping_mul(0x2545F4914F6CDD1D);
        rng
    };
    let hostile_f64 = |bits: u64| match bits % 9 {
        0 => f64::NAN,
        1 => f64::INFINITY,
        2 => f64::NEG_INFINITY,
        3 => -1e6,
        4 => -0.5,
        5 => 0.0,
        6 => 0.5,
        7 => 2.0,
        _ => 1e6,
    };
    for round in 0..200 {
        let start = (next_u64() as i64) % 10_000 - 5_000;
        let end = (next_u64() as i64) % 10_000 - 5_000;
        let state = EditorState {
            appearance: Appearance {
                background: if round % 3 == 0 {
                    String::new()
                } else {
                    "#fff".into()
                },
                padding: hostile_f64(next_u64()),
                border_radius: hostile_f64(next_u64()),
                shadow: hostile_f64(next_u64()),
                aspect_ratio: Some("bogus".into()),
            },
            zooms: vec![zoom(
                "z",
                start,
                end,
                (next_u64() % 10) as u32,
                hostile_f64(next_u64()),
                hostile_f64(next_u64()),
            )],
            clips: vec![clip("c", start, end, hostile_f64(next_u64()))],
            trims: vec![TrimRegion {
                id: "t".into(),
                start_ms: start,
                end_ms: end,
            }],
            speeds: vec![SpeedRegion {
                id: "s".into(),
                start_ms: start,
                end_ms: end,
                speed: hostile_f64(next_u64()),
            }],
            captions: vec![CaptionCue {
                id: "cap".into(),
                start_ms: start,
                end_ms: end,
                text: "x".into(),
                words: vec![CaptionWord {
                    text: "x".into(),
                    start_ms: end,
                    end_ms: start,
                }],
            }],
            annotations: vec![AnnotationRegion {
                id: "a".into(),
                start_ms: start,
                end_ms: end,
                kind: AnnotationKind::Text,
                content: "hi".into(),
                position: AnnotationPosition {
                    x: hostile_f64(next_u64()),
                    y: hostile_f64(next_u64()),
                },
                size: AnnotationSize {
                    width: hostile_f64(next_u64()),
                    height: hostile_f64(next_u64()),
                },
                style: AnnotationTextStyle {
                    font_size: hostile_f64(next_u64()),
                    border_radius: hostile_f64(next_u64()),
                    ..AnnotationTextStyle::default()
                },
                z_index: 0,
                figure: None,
                blur_intensity: Some(hostile_f64(next_u64())),
            }],
            audio_regions: vec![AudioRegion {
                id: "au".into(),
                start_ms: start,
                end_ms: end,
                audio_path: "media/a.wav".into(),
                volume: hostile_f64(next_u64()),
                normalize_audio: false,
                track_index: None,
            }],
            webcam: WebcamOverlay {
                position_x: hostile_f64(next_u64()),
                position_y: hostile_f64(next_u64()),
                size: hostile_f64(next_u64()),
                roundness: hostile_f64(next_u64()),
                shadow: hostile_f64(next_u64()),
                crop: CropRegion {
                    x: hostile_f64(next_u64()),
                    y: hostile_f64(next_u64()),
                    width: hostile_f64(next_u64()),
                    height: hostile_f64(next_u64()),
                },
                ..WebcamOverlay::default()
            },
            ..EditorState::default()
        };
        let once = state.normalized();
        let twice = once.clone().normalized();
        assert_eq!(once, twice, "EditorState not idempotent on round {round}");
    }
}
