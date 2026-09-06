use kiri_audio::{AudioSourceKind, enumerate_devices, start_recording};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let devices = enumerate_devices()?;
    println!("{}", serde_json::to_string_pretty(&devices)?);
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let mic = devices
        .iter()
        .find(|d| d.kind == AudioSourceKind::Microphone && d.is_default)
        .map(|d| d.id.clone());
    let a = mic
        .map(|id| {
            start_recording(
                AudioSourceKind::Microphone,
                Some(id),
                "target/microphone-diagnostic.wav".into(),
            )
        })
        .transpose()?;
    let b = start_recording(
        AudioSourceKind::SystemLoopback,
        None,
        "target/system-audio-diagnostic.wav".into(),
    )?;
    std::thread::sleep(std::time::Duration::from_secs(seconds));
    let mic_frames = a.map(|h| h.stop()).transpose()?.unwrap_or(0);
    let loop_frames = b.stop()?;
    println!("{{\"micFrames\":{mic_frames},\"loopbackFrames\":{loop_frames}}}");
    Ok(())
}
