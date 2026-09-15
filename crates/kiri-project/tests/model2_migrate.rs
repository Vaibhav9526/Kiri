//! Phase 2 hardening: migration of older/foreign payloads.
//!
//! Older manifests (v0/v1, snake_case, float micros, string versions) must
//! migrate without losing data. Foreign/future payloads must error cleanly
//! (no panic) and never overwrite the original file.

use kiri_project::{CURRENT_SCHEMA_VERSION, migrate, open_project};
use serde_json::json;

#[test]
fn migrates_v0_fixture_without_data_loss() {
    let value: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/schema-v0.json")).unwrap();
    let migrated = migrate(value).unwrap();
    assert_eq!(
        migrated.get("schemaVersion").and_then(|v| v.as_u64()),
        Some(2)
    );
    let project: kiri_project::ProjectManifest = serde_json::from_value(migrated).unwrap();
    project.validate().unwrap();
    assert_eq!(project.title, "Migrated fixture");
}

#[test]
fn migrates_snake_case_current_payload() {
    let id = uuid::Uuid::new_v4();
    let value = json!({
        "schemaVersion": CURRENT_SCHEMA_VERSION,
        "id": id,
        "title": "Snake",
        "createdAt": "2026-01-01T00:00:00Z",
        "updatedAt": "2026-01-01T00:00:00Z",
        "frame_rate": { "numerator": 30, "denominator": 1 },
        "duration": 0,
        "sources": [{ "id": uuid::Uuid::new_v4(), "kind": "screen_video", "relative_path": "media/a.mp4" }],
        "tracks": [{ "id": uuid::Uuid::new_v4(), "kind": "screen", "clips": [
            { "id": uuid::Uuid::new_v4(), "source_id": uuid::Uuid::new_v4(), "start": 0, "duration": 100, "source_offset": 0 }
        ]}],
        "edit_regions": [],
        "artifacts": [],
        "cache_entries": [],
        "recording_sessions": [],
    });
    let migrated = migrate(value).unwrap();
    let project: kiri_project::ProjectManifest = serde_json::from_value(migrated).unwrap();
    project.validate().unwrap();
}

#[test]
fn migrates_string_version_and_float_micros() {
    let value = json!({
        "schemaVersion": "1",
        "id": uuid::Uuid::new_v4(),
        "title": "Coerced",
        "createdAt": "2026-01-01T00:00:00Z",
        "updatedAt": "2026-01-01T00:00:00Z",
        "frameRate": { "numerator": 30, "denominator": 1 },
        "duration": 123.6,
    });
    let migrated = migrate(value).unwrap();
    assert_eq!(
        migrated.get("schemaVersion").and_then(|v| v.as_u64()),
        Some(u64::from(CURRENT_SCHEMA_VERSION))
    );
    let project: kiri_project::ProjectManifest = serde_json::from_value(migrated).unwrap();
    assert_eq!(project.duration.0, 124);
    project.validate().unwrap();
}

#[test]
fn migrate_is_total_over_garbage_roots() {
    // Non-object roots with missing version fall into the v0 path and must
    // return a Validation error, never panic.
    for value in [
        json!(null),
        json!(42),
        json!("hello"),
        json!([]),
        json!([{"schemaVersion": 1}]),
        json!({"schema_version": "not-a-number"}),
    ] {
        let result = migrate(value);
        // `"not-a-number"` maps to version 0 -> object path; the object with
        // missing required fields still migrates Ok (deserialization fails
        // later in open_project). Non-objects must Err.
        if result.is_ok() {
            let v = result.unwrap();
            assert!(v.is_object());
        }
    }
    // Future schemas reject with a typed error.
    for future in [3, 99, 999, u32::MAX] {
        let err = migrate(json!({"schemaVersion": future})).unwrap_err();
        assert!(
            matches!(err, kiri_project::ProjectError::UnsupportedSchema(v) if v == future),
            "wrong error for future {future}: {err}"
        );
    }
}

#[test]
fn foreign_recordly_payload_errors_without_panic_or_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("foreign.kiri");
    std::fs::create_dir_all(&root).unwrap();
    // A Recordly EditorProjectData shape: version + videoPath + editor.
    let foreign = json!({
        "version": 2,
        "videoPath": "C:\\Videos\\demo.mp4",
        "editor": { "zoomRegions": [], "clipRegions": [] },
    });
    std::fs::write(
        root.join("project.json"),
        serde_json::to_vec_pretty(&foreign).unwrap(),
    )
    .unwrap();
    let before = std::fs::read(root.join("project.json")).unwrap();
    let result = open_project(&root);
    assert!(
        result.is_err(),
        "foreign payload should not open as kiri manifest"
    );
    let after = std::fs::read(root.join("project.json")).unwrap();
    assert_eq!(
        before, after,
        "failed open must not touch the original file"
    );
}

#[test]
fn editor_aliases_migrate_without_failing_open() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("alias.kiri");
    let project = kiri_project::create_project(&root, "Alias").unwrap();
    project.validate().unwrap();
    // Write a current-version manifest whose editor uses Recordly key names.
    let mut raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("project.json")).unwrap()).unwrap();
    raw["editor"] = json!({
        "zoomRegions": [{ "id": "z1", "startMs": 900, "endMs": 100, "depth": 99, "focus": { "cx": 9.0, "cy": -2.0 } }],
        "clipRegions": [{ "id": "c1", "startMs": 800, "endMs": 200, "speed": 99.0 }],
        "trimRegions": [{ "id": "t1", "startMs": 5, "endMs": 1 }],
        "speedRegions": [{ "id": "s1", "startMs": 5, "endMs": 1, "speed": 9.0 }],
        "autoCaptions": [{ "id": "cap1", "startMs": 5, "endMs": 1, "text": "hi" }],
        "annotationRegions": [],
        "audioRegions": [],
    });
    std::fs::write(
        root.join("project.json"),
        serde_json::to_vec_pretty(&raw).unwrap(),
    )
    .unwrap();
    let reopened = open_project(&root).unwrap();
    // Aliased regions must survive (normalized, not dropped).
    assert_eq!(reopened.editor.zooms.len(), 1);
    assert_eq!(
        (
            reopened.editor.zooms[0].start_ms,
            reopened.editor.zooms[0].end_ms
        ),
        (100, 900)
    );
    assert_eq!(reopened.editor.clips.len(), 1);
    assert_eq!(reopened.editor.trims.len(), 1);
    assert_eq!(reopened.editor.speeds.len(), 1);
    assert_eq!(reopened.editor.captions.len(), 1);
}

#[test]
fn corrupt_manifest_bytes_error_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("corrupt.kiri");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("project.json"), b"{ not json").unwrap();
    assert!(open_project(&root).is_err());
    std::fs::write(root.join("project.json"), b"[1,2,3]").unwrap();
    assert!(open_project(&root).is_err());
}
