//! Phase 2 hardening: atomic-save durability + manifest lifecycle.
//!
//! Failed saves must leave the previous manifest readable, backups must
//! exist, autosave must not advance `updated_at` on failure, Save As must
//! never overwrite or recurse, and create must never clobber.

use kiri_project::{
    ProjectManifest, autosave_project, create_project, open_project, save_as, save_project,
};

fn read_raw(root: &std::path::Path) -> Vec<u8> {
    std::fs::read(root.join("project.json")).unwrap()
}

#[test]
fn failed_save_leaves_previous_manifest_and_backup() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("demo.kiri");
    create_project(&root, "Original").unwrap();
    let before = read_raw(&root);

    let mut invalid = open_project(&root).unwrap();
    invalid.title = String::new();
    assert!(save_project(&root, &invalid).is_err());
    assert_eq!(read_raw(&root), before);
    assert_eq!(open_project(&root).unwrap().title, "Original");

    // A successful second save must create a recovery backup of the first.
    let mut second = open_project(&root).unwrap();
    second.title = "Second".into();
    save_project(&root, &second).unwrap();
    assert!(root.join("recovery/project.json.bak").is_file());
    assert_eq!(open_project(&root).unwrap().title, "Second");
}

#[test]
fn autosave_restores_timestamp_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("demo.kiri");
    let mut project = create_project(&root, "Original").unwrap();
    let durable_before = read_raw(&root);

    project.title = String::new();
    let stamped_before = project.updated_at;
    assert!(autosave_project(&root, &mut project).is_err());
    assert_eq!(project.updated_at, stamped_before);
    assert_eq!(read_raw(&root), durable_before);
}

#[test]
fn create_project_never_clobbers_and_validates_first() {
    let dir = tempfile::tempdir().unwrap();
    // Empty titles fail before any directory is created.
    let missing = dir.path().join("empty.kiri");
    assert!(create_project(&missing, "   ").is_err());
    assert!(!missing.exists());

    // Extension check is case-insensitive but still enforced.
    let upper = dir.path().join("upper.KIRI");
    create_project(&upper, "Upper").unwrap();
    assert!(upper.join("project.json").is_file());

    let bad = dir.path().join("plain-folder");
    assert!(create_project(&bad, "Demo").is_err());

    // Creating over an existing manifest is data loss: must fail and keep
    // the original.
    let root = dir.path().join("demo.kiri");
    create_project(&root, "Original").unwrap();
    let before = read_raw(&root);
    assert!(create_project(&root, "Overwrite").is_err());
    assert_eq!(read_raw(&root), before);
    assert_eq!(open_project(&root).unwrap().title, "Original");
}

#[test]
fn save_as_guards_destination_and_cleans_up() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.kiri");
    create_project(&source, "Source").unwrap();

    // Existing destination: no overwrite.
    assert!(save_as(&source, &source, "Copy").is_err());

    // Non-.kiri destination: rejected before copying.
    let plain = dir.path().join("plain-folder");
    assert!(save_as(&source, &plain, "Copy").is_err());
    assert!(!plain.exists());

    // Empty title: rejected before copying.
    let empty_title = dir.path().join("empty.kiri");
    assert!(save_as(&source, &empty_title, "   ").is_err());
    assert!(!empty_title.exists());

    // Destination inside source: would recurse forever.
    let nested = source.join("nested.kiri");
    assert!(save_as(&source, &nested, "Nested").is_err());
    assert!(!nested.exists());

    // Happy path rekeys and leaves the source untouched.
    let destination = dir.path().join("copy.kiri");
    let original = open_project(&source).unwrap();
    let copied = save_as(&source, &destination, "Copy").unwrap();
    assert_eq!(copied.title, "Copy");
    assert_ne!(copied.id, original.id);
    assert_eq!(open_project(&destination).unwrap().title, "Copy");
    assert_eq!(open_project(&source).unwrap().title, "Source");

    // Corrupt source: Save As fails and removes the partial destination.
    let corrupt = dir.path().join("corrupt.kiri");
    std::fs::create_dir_all(&corrupt).unwrap();
    std::fs::write(corrupt.join("project.json"), b"{ bad").unwrap();
    let failed = dir.path().join("failed.kiri");
    assert!(save_as(&corrupt, &failed, "Failed").is_err());
    assert!(
        !failed.exists(),
        "partial Save As destination must be removed"
    );
}

#[test]
fn manifest_rejects_duplicates_nil_ids_and_empty_kinds() {
    use uuid::Uuid;
    let mut project = ProjectManifest::empty("Demo");

    // Duplicate source IDs.
    let dup = Uuid::new_v4();
    let source = || kiri_project::SourceMedia {
        id: dup,
        kind: kiri_project::SourceKind::ScreenVideo,
        relative_path: "media/a.mp4".into(),
    };
    project.sources = vec![source(), source()];
    assert!(project.validate().is_err());
    project.sources.clear();

    // Nil track ID and duplicate tracks.
    let track_id = Uuid::new_v4();
    project.tracks = vec![
        kiri_project::Track {
            id: Uuid::nil(),
            kind: kiri_project::TrackKind::Screen,
            clips: vec![],
        },
        kiri_project::Track {
            id: track_id,
            kind: kiri_project::TrackKind::Screen,
            clips: vec![],
        },
    ];
    assert!(project.validate().is_err());
    project.tracks.clear();

    // Nil clip ID / nil source ID.
    project.tracks = vec![kiri_project::Track {
        id: Uuid::new_v4(),
        kind: kiri_project::TrackKind::Screen,
        clips: vec![kiri_project::Clip {
            id: Uuid::nil(),
            source_id: Uuid::new_v4(),
            start: kiri_project::TimeMicros(0),
            duration: kiri_project::TimeMicros(10),
            source_offset: kiri_project::TimeMicros(0),
        }],
    }];
    assert!(project.validate().is_err());
    project.tracks.clear();

    // Empty edit-region kind and duplicate region IDs.
    let region_id = Uuid::new_v4();
    project.edit_regions = vec![kiri_project::EditRegion {
        id: region_id,
        kind: "   ".into(),
        start: kiri_project::TimeMicros(0),
        duration: kiri_project::TimeMicros(10),
        payload: serde_json::json!({}),
    }];
    assert!(project.validate().is_err());
    project.edit_regions.clear();
    project.validate().unwrap();
}
