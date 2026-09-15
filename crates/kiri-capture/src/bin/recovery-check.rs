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
    let recovery = kiri_capture::RecoveryManifest::replay(&root)?
        .ok_or_else(|| invalid_input("recovery manifest missing"))?;
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

#[cfg(test)]
mod tests {
    #[test]
    fn missing_arg_message_is_actionable() {
        let error = super::invalid_input("project root required");
        assert!(error.to_string().contains("project root required"));
    }
}
