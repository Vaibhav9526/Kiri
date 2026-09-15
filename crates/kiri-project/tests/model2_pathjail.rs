//! Phase 2 hardening: path-jail escapes.
//!
//! `validate_relative_path` must keep every accepted path inside the project
//! on Windows. Backslashes are normalized; `..`, absolute paths, prefixes,
//! roots, ADS colons, NUL, glob/shell metacharacters, device names, and
//! trailing dots/spaces must all reject.

use kiri_project::validate_relative_path;

#[test]
fn accepts_ordinary_collected_assets() {
    for ok in [
        "media/screen-001.mp4",
        "telemetry/cursor.jsonl",
        "transcript/original.json",
        "assets/logo.png",
        "cache/proxies/p1.mp4",
        "exports/final.mp4",
        "media\\screen-0001.mp4",
        "a/b/c/d.wav",
    ] {
        assert!(validate_relative_path(ok).is_ok(), "should accept {ok}");
    }
}

#[test]
fn rejects_dotdot_escapes_in_all_forms() {
    for evil in [
        "../secret.txt",
        "..\\secret.txt",
        "a/../../b.mp4",
        "a\\..\\..\\b.mp4",
        "media/../secret.mp4",
        "media/..\\secret.mp4",
        "..",
        "../..",
        "a/b/../../../etc/passwd",
    ] {
        assert!(
            validate_relative_path(evil).is_err(),
            "should reject {evil}"
        );
    }
}

#[test]
fn rejects_absolute_and_prefixed_paths() {
    for evil in [
        "/absolute/path.mp4",
        "\\absolute\\path.mp4",
        "C:/evil.mp4",
        "C:\\evil.mp4",
        "C:evil.mp4",
        "\\\\server\\share\\evil.mp4",
        "//server/share/evil.mp4",
        "\\\\?\\C:\\evil.mp4",
        "file:///etc/passwd",
    ] {
        // `file:///...` has no backslashes/colons after normalization? It
        // contains a colon so it must reject via the colon rule.
        assert!(
            validate_relative_path(evil).is_err(),
            "should reject {evil}"
        );
    }
}

#[test]
fn rejects_ads_colons_nul_and_shell_characters() {
    for evil in [
        "media/evil:stream",
        "media/evil.mp4:secret",
        "media/\0evil.mp4",
        "media/a*b.mp4",
        "media/a?b.mp4",
        "media/a<b.mp4",
        "media/a>b.mp4",
        "media/a|b.mp4",
        "media/a\"b.mp4",
    ] {
        assert!(
            validate_relative_path(evil).is_err(),
            "should reject {evil:?}"
        );
    }
}

#[test]
fn rejects_dot_segments_device_names_and_trailing_dots() {
    for evil in [
        ".",
        "./evil.mp4",
        "media/./evil.mp4",
        "media/CON",
        "media/con.mp4",
        "media/PRN.txt",
        "media/AUX",
        "media/NUL.mp4",
        "media/COM1",
        "media/com9.txt",
        "media/LPT1.mp4",
        "media/evil.",
        "media/evil ",
        "",
    ] {
        assert!(
            validate_relative_path(evil).is_err(),
            "should reject {evil:?}"
        );
    }
}

#[test]
fn backslash_normalization_cannot_smuggle_escapes() {
    // After `\` -> `/` normalization these all contain `..` and must fail.
    assert!(validate_relative_path("media\\..\\..\\secret").is_err());
    assert!(validate_relative_path("..\\..\\secret").is_err());
    // A plain backslash asset path is fine and maps inside the project.
    let ok = validate_relative_path("media\\sub\\file.mp4").unwrap();
    assert!(ok.to_str().unwrap().contains("media"));
}
