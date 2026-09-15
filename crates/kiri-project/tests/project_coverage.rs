use kiri_project::{
    FrameRate, ProjectManifest, TimeMicros, create_project, migrate, open_project, save_as,
    save_project, validate_relative_path,
};
use uuid::Uuid;

#[test]
fn relative_paths_stay_inside_the_project() {
    assert!(validate_relative_path("media/screen-0001.mp4").is_ok());
    assert!(validate_relative_path("telemetry/cursor.jsonl").is_ok());
    // Backslashes are normalized to forward slashes.
    assert!(validate_relative_path("media\\screen-0001.mp4").is_ok());
    assert!(validate_relative_path("").is_err());
    assert!(validate_relative_path("../secret.txt").is_err());
    assert!(validate_relative_path("a/../../b.mp4").is_err());
    assert!(validate_relative_path("/absolute/path.mp4").is_err());
}

#[test]
fn time_and_frame_rate_validation_reject_degenerate_values() {
    assert!(TimeMicros(0).validate().is_ok());
    assert!(TimeMicros(-1).validate().is_err());
    assert!(
        FrameRate {
            numerator: 30,
            denominator: 1
        }
        .validate()
        .is_ok()
    );
    assert!(
        FrameRate {
            numerator: 0,
            denominator: 1
        }
        .validate()
        .is_err()
    );
    assert!(
        FrameRate {
            numerator: 30,
            denominator: 0
        }
        .validate()
        .is_err()
    );
}

#[test]
fn manifest_rejects_empty_titles_and_nil_ids() {
    let mut project = ProjectManifest::empty("Demo");
    project.validate().unwrap();
    project.title = "   ".into();
    assert!(project.validate().is_err());
    project.title = "Demo".into();
    project.id = Uuid::nil();
    assert!(project.validate().is_err());
}

#[test]
fn create_project_requires_a_kiri_directory_and_lays_out_folders() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("plain-folder");
    assert!(create_project(&bad, "Demo").is_err());

    let root = dir.path().join("demo.kiri");
    let manifest = create_project(&root, "Demo").unwrap();
    assert_eq!(manifest.title, "Demo");
    assert!(root.join("project.json").is_file());
    for folder in ["media", "telemetry", "recovery", "exports"] {
        assert!(root.join(folder).is_dir(), "missing {folder}");
    }
    let reopened = open_project(&root).unwrap();
    assert_eq!(reopened.id, manifest.id);
}

#[test]
fn failed_save_keeps_the_previous_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("demo.kiri");
    create_project(&root, "Original").unwrap();
    let mut invalid = open_project(&root).unwrap();
    invalid.title = String::new();
    assert!(save_project(&root, &invalid).is_err());
    assert_eq!(open_project(&root).unwrap().title, "Original");
}

#[test]
fn save_as_rejects_existing_destinations_and_rekeys_the_copy() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.kiri");
    create_project(&source, "Source").unwrap();
    let original = open_project(&source).unwrap();

    assert!(save_as(&source, &source, "Copy").is_err());

    let destination = dir.path().join("copy.kiri");
    let copied = save_as(&source, &destination, "Copy").unwrap();
    assert_eq!(copied.title, "Copy");
    assert_ne!(copied.id, original.id);
    assert_eq!(open_project(&destination).unwrap().title, "Copy");
    // The source project is untouched by Save As.
    assert_eq!(open_project(&source).unwrap().title, "Source");
}

#[test]
fn migrate_rejects_unknown_future_schemas() {
    let value = serde_json::json!({
        "schemaVersion": 999,
        "id": Uuid::new_v4(),
        "title": "Future",
    });
    assert!(migrate(value).is_err());
}

#[test]
fn recording_sessions_with_bad_clocks_are_rejected() {
    let mut project = ProjectManifest::empty("Demo");
    project
        .recording_sessions
        .push(kiri_project::RecordingSessionMetadata {
            id: Uuid::nil(),
            wall_time_utc: chrono::Utc::now(),
            qpc_origin_ticks: 0,
            qpc_frequency: 0,
            paused_duration: TimeMicros(0),
            interrupted: true,
        });
    assert!(project.validate().is_err());
}

#[test]
fn clips_with_negative_times_are_rejected() {
    let mut project = ProjectManifest::empty("Demo");
    let source_id = Uuid::new_v4();
    project.tracks.push(kiri_project::Track {
        id: Uuid::new_v4(),
        kind: kiri_project::TrackKind::Screen,
        clips: vec![kiri_project::Clip {
            id: Uuid::new_v4(),
            source_id,
            start: TimeMicros(-5),
            duration: TimeMicros(100),
            source_offset: TimeMicros(0),
        }],
    });
    assert!(project.validate().is_err());
}
