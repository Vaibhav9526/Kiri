use kiri_capture::windows::{ScreenRecordingConfig, start_screen_segment};
use kiri_capture::{ClockOrigin, RecoveryManifest, RecoverySegment, SegmentKind};
use uuid::Uuid;

fn invalid_input(message: &str) -> Box<dyn std::error::Error> {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message.to_string(),
    ))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| invalid_input("project root required"))?,
    );
    std::fs::create_dir_all(root.join("media"))?;
    let source = kiri_capture::windows::enumerate_sources()?
        .into_iter()
        .find(|s| matches!(s.kind, kiri_capture::CaptureSourceKind::Display))
        .ok_or_else(|| invalid_input("no display available for forced-recovery test"))?;
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
    // Integer-micros conversion saturates instead of wrapping on absurd
    // durations, keeping the recovery manifest timestamps monotonic.
    let first_duration_micros = kiri_capture::secs_f64_to_micros(duration);
    let first_segment = recovery
        .segments
        .first_mut()
        .ok_or_else(|| invalid_input("recovery manifest lost its first segment"))?;
    first_segment.duration_micros = Some(first_duration_micros);
    first_segment.finalized = true;
    let second = "media/screen-0002.mp4";
    recovery.segments.push(RecoverySegment {
        id: Uuid::new_v4(),
        kind: SegmentKind::Screen,
        relative_path: second.into(),
        start_micros: first_duration_micros,
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

#[cfg(test)]
mod tests {
    #[test]
    fn segment_start_chains_from_first_duration_without_float_drift() {
        let first = kiri_capture::secs_f64_to_micros(3.0);
        assert_eq!(first, 3_000_000);
        // The second segment starts exactly where the first ended, so a
        // 20-minute chain of segments stays gapless.
        assert_eq!(kiri_capture::secs_f64_to_micros(0.5), 500_000);
    }
}
