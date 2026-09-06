fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sources = kiri_capture::windows::enumerate_sources()?;
    println!("{}", serde_json::to_string_pretty(&sources)?);
    if let Some(source) = sources.iter().find(|source| {
        matches!(
            source.availability,
            kiri_capture::SourceAvailability::Available
        )
    }) {
        let thumbnail = kiri_capture::windows::capture_thumbnail(&source.id)?;
        println!("thumbnailSource={} bytes={}", source.id, thumbnail.len());
    }
    Ok(())
}
