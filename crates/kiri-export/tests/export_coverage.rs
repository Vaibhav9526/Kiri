use kiri_export::{
    EncodingMode, ExportCancelToken, ExportPreset, ExportRequest, detect_silences,
    is_ffmpeg_progress_end, is_valid_mp4_frame_rate, parse_ffmpeg_frame_count,
    parse_ffmpeg_out_time_ms, parse_ffmpeg_progress_line, plan_export, target_bitrate_bps,
};
use std::path::PathBuf;
use uuid::Uuid;

fn request() -> ExportRequest {
    ExportRequest {
        project_id: Uuid::new_v4(),
        preset: ExportPreset::FullHd,
        fps: 60,
        encoding_mode: EncodingMode::Balanced,
        video_input: PathBuf::from("media/screen-0001.mp4"),
        audio_inputs: vec![PathBuf::from("media/microphone-0001.wav")],
        output: PathBuf::from("exports/walkthrough.mp4"),
    }
}

#[test]
fn presets_define_dimensions_and_bitrates() {
    assert_eq!(ExportPreset::FullHd.dimensions(), (1920, 1080));
    assert_eq!(ExportPreset::Hd.dimensions(), (1280, 720));
    assert_eq!(ExportPreset::FullHd.video_bitrate(), "12M");
    assert_eq!(ExportPreset::Hd.video_bitrate(), "8M");
    assert_eq!(ExportPreset::default(), ExportPreset::FullHd);
}

#[test]
fn preset_serde_uses_resolution_names() {
    assert_eq!(
        serde_json::from_str::<ExportPreset>("\"1080p\"").unwrap(),
        ExportPreset::FullHd
    );
    assert_eq!(
        serde_json::from_str::<ExportPreset>("\"720p\"").unwrap(),
        ExportPreset::Hd
    );
}

#[test]
fn mp4_policy_accepts_24_30_60_only() {
    for fps in [24, 30, 60] {
        assert!(is_valid_mp4_frame_rate(fps), "fps {fps} should be valid");
        let mut req = request();
        req.fps = fps;
        assert!(plan_export(&req).is_ok());
    }
    for fps in [0, 1, 23, 25, 29, 31, 59, 61, 120] {
        assert!(!is_valid_mp4_frame_rate(fps), "fps {fps} should be invalid");
    }
    let mut bad = request();
    bad.fps = 23;
    assert!(plan_export(&bad).is_err());
}

#[test]
fn validation_rejects_missing_inputs_outputs_and_extensions() {
    let mut no_video = request();
    no_video.video_input = PathBuf::from("");
    assert!(plan_export(&no_video).is_err());

    let mut no_output = request();
    no_output.output = PathBuf::from("");
    assert!(plan_export(&no_output).is_err());

    for bad in [
        "exports/walkthrough.mkv",
        "exports/walkthrough",
        "exports/walkthrough.MP3",
    ] {
        let mut req = request();
        req.output = PathBuf::from(bad);
        assert!(plan_export(&req).is_err(), "{bad} should be rejected");
    }
    // Extension checks are ASCII case-insensitive.
    let mut upper = request();
    upper.output = PathBuf::from("exports/walkthrough.MP4");
    assert!(plan_export(&upper).is_ok());

    let mut too_many = request();
    too_many.audio_inputs = (0..9).map(|i| PathBuf::from(format!("a{i}.wav"))).collect();
    assert!(plan_export(&too_many).is_err());
    too_many.audio_inputs.pop();
    assert!(plan_export(&too_many).is_ok());
}

#[test]
fn old_requests_without_encoding_mode_still_parse_with_balanced_default() {
    let value = serde_json::json!({
        "projectId": Uuid::new_v4(),
        "preset": "1080p",
        "fps": 30,
        "videoInput": "media/screen-0001.mp4",
        "audioInputs": [],
        "output": "exports/walkthrough.mp4",
    });
    let parsed: ExportRequest = serde_json::from_value(value).unwrap();
    assert_eq!(parsed.encoding_mode, EncodingMode::Balanced);
    assert!(plan_export(&parsed).is_ok());
}

#[test]
fn plan_is_deterministic_h264_aac_faststart() {
    let first = plan_export(&request()).unwrap();
    let second = plan_export(&request()).unwrap();
    assert_eq!(first.ffmpeg_args, second.ffmpeg_args);
    let args = &first.ffmpeg_args;
    assert!(args.contains(&"libx264".to_string()));
    assert!(args.contains(&"aac".to_string()));
    assert!(args.contains(&"+faststart".to_string()));
    // Balanced default keeps the golden preset/CRF pair.
    let preset_pos = args.iter().position(|a| a == "-preset").unwrap();
    assert_eq!(args[preset_pos + 1], "medium");
    let crf_pos = args.iter().position(|a| a == "-crf").unwrap();
    assert_eq!(args[crf_pos + 1], "18");
    assert_eq!((first.width, first.height), (1920, 1080));
}

#[test]
fn encoding_modes_select_expected_presets() {
    for (mode, preset, crf) in [
        (EncodingMode::Fast, "veryfast", "20"),
        (EncodingMode::Balanced, "medium", "18"),
        (EncodingMode::Quality, "slow", "16"),
    ] {
        let mut req = request();
        req.encoding_mode = mode;
        let plan = plan_export(&req).unwrap();
        let preset_pos = plan
            .ffmpeg_args
            .iter()
            .position(|a| a == "-preset")
            .unwrap();
        assert_eq!(plan.ffmpeg_args[preset_pos + 1], preset);
        let crf_pos = plan.ffmpeg_args.iter().position(|a| a == "-crf").unwrap();
        assert_eq!(plan.ffmpeg_args[crf_pos + 1], crf);
    }
}

#[test]
fn windows_paths_are_normalized_to_forward_slashes() {
    let mut req = request();
    req.video_input = PathBuf::from("media\\screen-0001.mp4");
    req.output = PathBuf::from("exports\\walkthrough.mp4");
    let plan = plan_export(&req).unwrap();
    assert!(
        plan.ffmpeg_args
            .contains(&"media/screen-0001.mp4".to_string())
    );
    assert!(
        plan.ffmpeg_args
            .contains(&"exports/walkthrough.mp4".to_string())
    );
}

#[test]
fn multiple_audio_inputs_each_add_an_input_flag() {
    let mut req = request();
    req.audio_inputs = vec![
        PathBuf::from("media/mic.wav"),
        PathBuf::from("media/system.wav"),
    ];
    let plan = plan_export(&req).unwrap();
    let inputs = plan
        .ffmpeg_args
        .iter()
        .filter(|a| a.as_str() == "-i")
        .count();
    assert_eq!(inputs, 3); // 1 video + 2 audio
}

#[test]
fn target_bitrate_scales_with_fps_and_mode() {
    let base_30 = target_bitrate_bps(ExportPreset::FullHd, 30, EncodingMode::Quality);
    assert_eq!(base_30, 12_000_000);
    let base_60 = target_bitrate_bps(ExportPreset::FullHd, 60, EncodingMode::Quality);
    assert!(base_60 > base_30);
    assert!(
        target_bitrate_bps(ExportPreset::FullHd, 30, EncodingMode::Fast)
            < target_bitrate_bps(ExportPreset::FullHd, 30, EncodingMode::Balanced)
    );
    // Floored at 2 Mbps even for tiny presets.
    assert!(target_bitrate_bps(ExportPreset::Hd, 1, EncodingMode::Fast) >= 2_000_000);
}

#[test]
fn progress_parsers_handle_pipe_and_stderr_lines() {
    assert_eq!(parse_ffmpeg_frame_count("frame=  240 fps=60"), Some(240));
    assert_eq!(parse_ffmpeg_frame_count("frame=123"), Some(123));
    assert_eq!(parse_ffmpeg_frame_count("nothing here"), None);

    assert_eq!(parse_ffmpeg_out_time_ms("out_time_ms=1500"), Some(1500));
    assert_eq!(parse_ffmpeg_out_time_ms("out_time_us=2500000"), Some(2500));
    assert_eq!(
        parse_ffmpeg_out_time_ms("frame=10 time=00:00:02.50 bitrate=1.0"),
        Some(2500)
    );
    assert_eq!(parse_ffmpeg_out_time_ms("time=N/A"), None);

    let continued = parse_ffmpeg_progress_line("progress=continue").unwrap();
    assert!(!continued.finished);
    let ended = parse_ffmpeg_progress_line("progress=end").unwrap();
    assert!(ended.finished);
    assert!(parse_ffmpeg_progress_line("   ").is_none());
    assert!(is_ffmpeg_progress_end("progress=end"));
    assert!(is_ffmpeg_progress_end("PROGRESS=END"));
    assert!(!is_ffmpeg_progress_end("progress=continue"));
}

#[test]
fn cancel_token_is_shared_and_cooperative() {
    let token = ExportCancelToken::new();
    assert!(!token.is_cancelled());
    let shared = token.clone();
    token.cancel();
    assert!(token.is_cancelled());
    assert!(shared.is_cancelled());
    assert_eq!(
        format!("{}", kiri_export::ExportError::Cancelled),
        "export cancelled"
    );
}

#[test]
fn silence_detection_reports_only_long_quiet_runs() {
    // 10 windows of 100ms: loud, 5 quiet, loud x4 -> one 500ms silence.
    let peaks = [0.9, 0.01, 0.02, 0.0, 0.03, 0.01, 0.8, 0.7, 0.9, 0.85];
    let found = detect_silences(&peaks, 100, 0.1, 400);
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].start_ms, found[0].end_ms), (100, 600));

    // Short blips below the minimum duration are ignored.
    assert!(detect_silences(&peaks, 100, 0.1, 600).is_empty());
    // Degenerate inputs are total (empty, never panic).
    assert!(detect_silences(&[], 100, 0.1, 400).is_empty());
    assert!(detect_silences(&peaks, 0, 0.1, 400).is_empty());
    assert!(detect_silences(&peaks, 100, 0.1, 0).is_empty());
    // Non-finite peaks count as sound, never silence.
    assert!(detect_silences(&[f32::NAN, f32::NAN, f32::NAN], 100, 0.5, 200).is_empty());
}
