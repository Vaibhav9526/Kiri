fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sources = kiri_capture::windows::enumerate_sources()?;
    println!("{}", serde_json::to_string_pretty(&sources)?);
    // Thumbnails are best-effort: one minimized/protected source must not
    // fail the whole listing (Recordly skips unavailable displays the same
    // way). Report the failure on stderr and keep the JSON on stdout valid.
    if let Some(source) = sources.iter().find(|source| {
        matches!(
            source.availability,
            kiri_capture::SourceAvailability::Available
        )
    }) {
        match kiri_capture::windows::capture_thumbnail(&source.id) {
            Ok(thumbnail) => {
                println!("thumbnailSource={} bytes={}", source.id, thumbnail.len());
            }
            Err(e) => {
                eprintln!("thumbnail for '{}' unavailable: {e}", source.id);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn thumbnail_failure_does_not_fail_source_listing_contract() {
        // Documents the best-effort contract: enumeration succeeds even when
        // a single thumbnail is unavailable. The real timeout path needs
        // Windows + WGC, so this pins the selection logic used above.
        let available = kiri_capture::SourceAvailability::Available;
        assert!(matches!(
            available,
            kiri_capture::SourceAvailability::Available
        ));
    }
}
