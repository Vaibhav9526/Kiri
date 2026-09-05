fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::PathBuf::from(std::env::args().nth(1).ok_or("root required")?);
    let recovery = kiri_capture::RecoveryManifest::replay(&root)?.ok_or("manifest missing")?;
    let playable = recovery.playable_segments(&root);
    println!(
        "active={} finalizedPlayable={} total={}",
        recovery.active,
        playable.len(),
        recovery.segments.len()
    );
    if playable.is_empty() {
        std::process::exit(2)
    }
    Ok(())
}
