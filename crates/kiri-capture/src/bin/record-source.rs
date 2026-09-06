use kiri_capture::windows::{ScreenRecordingConfig, start_screen_segment};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let source_id = args.next().ok_or("source id required")?;
    let output = args
        .next()
        .unwrap_or_else(|| "source-diagnostic.mp4".into());
    let seconds: u64 = args.next().and_then(|v| v.parse().ok()).unwrap_or(5);
    let fps: u32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(60);
    let handle = start_screen_segment(ScreenRecordingConfig {
        source_id,
        output: output.into(),
        fps,
        bitrate: 16_000_000,
    })?;
    std::thread::sleep(std::time::Duration::from_secs(seconds));
    let (frames, dropped, duration) = handle.stop()?;
    println!(
        "{{\"frames\":{frames},\"dropped\":{dropped},\"duration\":{duration:.3},\"fps\":{:.3}}}",
        frames as f64 / duration
    );
    Ok(())
}
