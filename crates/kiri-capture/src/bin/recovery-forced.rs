use kiri_capture::windows::{ScreenRecordingConfig, start_screen_segment};
use kiri_capture::{ClockOrigin, RecoveryManifest, RecoverySegment, SegmentKind};
use uuid::Uuid;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::PathBuf::from(std::env::args().nth(1).ok_or("root required")?);
    std::fs::create_dir_all(root.join("media"))?;
    let source = kiri_capture::windows::enumerate_sources()?
        .into_iter()
        .find(|s| matches!(s.kind, kiri_capture::CaptureSourceKind::Display))
        .ok_or("no display")?;
    let mut recovery = RecoveryManifest::new(Uuid::new_v4(), ClockOrigin::now()?);
    let first = "media/screen-0001.mp4";
    recovery.segments.push(RecoverySegment {
        id: Uuid::new_v4(),
        kind: SegmentKind::Screen,
        relative_path: first.into(),
        start_micros: 0,
        duration_micros: None,
        finalized: false,
    });
    recovery.commit(&root)?;
    let capture = start_screen_segment(ScreenRecordingConfig {
        source_id: source.id.clone(),
        output: root.join(first),
        fps: 30,
        bitrate: 10_000_000,
    })?;
    std::thread::sleep(std::time::Duration::from_secs(3));
    let (_, _, duration) = capture.stop()?;
    recovery.segments[0].duration_micros = Some((duration * 1_000_000.0) as i64);
    recovery.segments[0].finalized = true;
    let second = "media/screen-0002.mp4";
    recovery.segments.push(RecoverySegment {
        id: Uuid::new_v4(),
        kind: SegmentKind::Screen,
        relative_path: second.into(),
        start_micros: (duration * 1_000_000.0) as i64,
        duration_micros: None,
        finalized: false,
    });
    recovery.commit(&root)?;
    let _capture = start_screen_segment(ScreenRecordingConfig {
        source_id: source.id,
        output: root.join(second),
        fps: 30,
        bitrate: 10_000_000,
    })?;
    println!("READY {}", std::process::id());
    std::thread::sleep(std::time::Duration::from_secs(120));
    Ok(())
}
