use kiri_camera::{enumerate_devices, start_recording};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let devices = enumerate_devices()?;
    println!("{}", serde_json::to_string_pretty(&devices)?);
    let device = devices.first().ok_or("no camera available")?;
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(5);
    let handle = start_recording(device.id.clone(), "target/camera-diagnostic.mp4".into())?;
    std::thread::sleep(std::time::Duration::from_secs(seconds));
    let frames = handle.stop()?;
    println!("{{\"frames\":{frames},\"device\":{:?}}}", device.name);
    Ok(())
}
