use kiri_capture::windows::{ScreenRecordingConfig, start_screen_segment};

fn invalid_input(message: String) -> Box<dyn std::error::Error> {
    Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message,
    ))
}

/// Parses an optional CLI value: missing input yields the default, but a
/// present-but-invalid value is an error instead of silently recording with
/// the wrong duration/FPS.
fn parse_optional<T>(
    value: Option<String>,
    default: T,
    name: &str,
) -> Result<T, Box<dyn std::error::Error>>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match value {
        None => Ok(default),
        Some(text) => text
            .parse()
            .map_err(|e| invalid_input(format!("invalid {name} '{text}': {e}; check CLI usage"))),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let source_id = args
        .next()
        .ok_or_else(|| invalid_input("source id required".into()))?;
    let output: String = args
        .next()
        .unwrap_or_else(|| "source-diagnostic.mp4".into());
    let seconds: u64 = parse_optional(args.next(), 5, "seconds")?;
    let fps: u32 = parse_optional(args.next(), 60, "fps")?;
    if seconds == 0 || seconds > 3600 {
        return Err(invalid_input(format!(
            "invalid seconds '{seconds}'; expected 1..=3600"
        )));
    }
    let output_path = std::path::PathBuf::from(&output);
    let handle = start_screen_segment(ScreenRecordingConfig {
        source_id,
        output: output_path.clone(),
        fps,
        bitrate: 16_000_000,
    })?;
    std::thread::sleep(std::time::Duration::from_secs(seconds));
    let (frames, dropped, duration) = handle.stop()?;
    // Catch zero-byte/unplayable outputs at the CLI boundary (Recordly's
    // `validateRecordedVideo` rejects < 1024 bytes) instead of reporting a
    // successful-looking frame count for a corrupt file.
    let bytes = std::fs::metadata(&output_path).map(|meta| meta.len());
    let effective_fps = if duration > 0.0 {
        frames as f64 / duration
    } else {
        0.0
    };
    println!(
        "{{\"frames\":{frames},\"dropped\":{dropped},\"duration\":{duration:.3},\"fps\":{effective_fps:.3},\"bytes\":{}}}",
        match bytes {
            Ok(size) => size.to_string(),
            Err(_) => "null".into(),
        }
    );
    match bytes {
        Ok(size) if size >= kiri_capture::MIN_VALID_MEDIA_SEGMENT_BYTES => Ok(()),
        Ok(size) => Err(invalid_input(format!(
            "recorded output is too small to be playable ({size} bytes): {}",
            output_path.display()
        ))),
        Err(e) => Err(Box::new(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_optional;

    #[test]
    fn missing_cli_value_uses_default_but_invalid_value_errors() {
        let fallback: u64 = parse_optional(None, 5, "seconds").unwrap();
        assert_eq!(fallback, 5);
        assert!(parse_optional(Some("abc".into()), 5_u64, "seconds").is_err());
        assert_eq!(
            parse_optional(Some("7".into()), 5_u64, "seconds").unwrap(),
            7
        );
    }

    #[test]
    fn zero_duration_fps_reports_zero_instead_of_infinity() {
        let frames = 10_u64;
        let duration = 0.0_f64;
        let effective = if duration > 0.0 {
            frames as f64 / duration
        } else {
            0.0
        };
        assert_eq!(effective, 0.0);
        assert!(effective.is_finite());
    }
}
