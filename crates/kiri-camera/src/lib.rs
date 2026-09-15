use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CameraError {
    #[error("camera capture is only available on Windows")]
    Unsupported,
    #[error("camera unavailable: {0}")]
    Unavailable(String),
    #[error("timed out waiting for camera to start: {0}")]
    StartTimeout(String),
    #[error("camera device failed: {0}")]
    Device(String),
}

/// Mirrors the audio start timeout so a wedged camera open cannot hang the
/// Tauri command forever (Recordly uses 12s for native capture start).
pub const CAMERA_START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(12);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraDevice {
    pub id: String,
    pub name: String,
    pub formats: Vec<CameraFormat>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraFormat {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

/// Sort + dedupe camera formats for stable enumeration output.
#[must_use]
pub fn dedupe_and_sort_formats(mut formats: Vec<CameraFormat>) -> Vec<CameraFormat> {
    formats.sort_by_key(|format| (format.width, format.height, format.fps));
    formats.dedup();
    formats
}

/// Pick the format closest to 1280x720@30 (the Phase 1 default).
/// Pure helper so rapid start/stop + format-selection logic is unit-testable
/// without camera hardware.
#[must_use]
pub fn pick_best_format(formats: &[CameraFormat]) -> Option<CameraFormat> {
    formats
        .iter()
        .min_by_key(|format| {
            format.width.abs_diff(1280) + format.height.abs_diff(720) + format.fps.abs_diff(30) * 20
        })
        .copied()
}

/// Encoder frame rate derived from the camera's actual format. A zero fps
/// report (some virtual cameras) falls back to 30; otherwise clamp to the
/// 1..=60 range the Media Foundation H.264 path supports.
#[must_use]
pub fn select_encoder_fps(actual_fps: u32) -> u32 {
    if actual_fps == 0 {
        30
    } else {
        actual_fps.clamp(1, 60)
    }
}

/// Validate dimensions before creating the encoder so a zero-area source
/// becomes a clear error instead of a native panic.
pub fn validate_dimensions(width: u32, height: u32) -> Result<(), CameraError> {
    if width == 0 || height == 0 {
        return Err(CameraError::Device(
            "camera reported no visible area (0x0); it may be unplugged or exclusively locked"
                .into(),
        ));
    }
    Ok(())
}

/// Media Foundation timestamps are 100ns units. Saturates instead of
/// wrapping on very long captures.
#[must_use]
pub fn hns_from_elapsed(elapsed: std::time::Duration) -> i64 {
    (elapsed.as_nanos() / 100).min(i64::MAX as u128) as i64
}

/// Convert top-down RGBA to bottom-up BGRA (the `windows-capture` encoder
/// layout). Validates lengths so a short driver buffer becomes a clear
/// device error instead of a panicking capture thread.
pub fn convert_rgba_to_bgra_bottom_up(
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, CameraError> {
    let width = width as usize;
    let height = height as usize;
    if width == 0 || height == 0 {
        return Err(CameraError::Device(
            "camera reported no visible area (0x0)".into(),
        ));
    }
    let row_bytes = width.checked_mul(4).ok_or_else(|| {
        CameraError::Device("camera frame dimensions overflowed the frame buffer".into())
    })?;
    let expected = row_bytes.checked_mul(height).ok_or_else(|| {
        CameraError::Device("camera frame dimensions overflowed the frame buffer".into())
    })?;
    if rgba.len() != expected {
        return Err(CameraError::Device(format!(
            "camera frame was {} bytes but {}x{} RGBA needs {expected}",
            rgba.len(),
            width,
            height
        )));
    }
    let mut out = vec![0_u8; expected];
    for y in 0..height {
        let source = &rgba[y * row_bytes..(y + 1) * row_bytes];
        let destination_y = height - 1 - y;
        let destination = &mut out[destination_y * row_bytes..(destination_y + 1) * row_bytes];
        for (source_pixel, destination_pixel) in
            source.chunks_exact(4).zip(destination.chunks_exact_mut(4))
        {
            destination_pixel.copy_from_slice(&[
                source_pixel[2],
                source_pixel[1],
                source_pixel[0],
                source_pixel[3],
            ]);
        }
    }
    Ok(out)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraMetadata {
    pub mirrored_in_preview: bool,
    pub mirror_baked_into_source: bool,
}

impl Default for CameraMetadata {
    fn default() -> Self {
        Self {
            mirrored_in_preview: true,
            mirror_baked_into_source: false,
        }
    }
}

#[cfg(windows)]
pub fn enumerate_devices() -> Result<Vec<CameraDevice>, CameraError> {
    use nokhwa::{
        Camera,
        pixel_format::RgbAFormat,
        query,
        utils::{ApiBackend, CameraIndex, RequestedFormat, RequestedFormatType},
    };
    let mut result = Vec::new();
    for device in
        query(ApiBackend::MediaFoundation).map_err(|e| CameraError::Device(e.to_string()))?
    {
        let id = device.index().to_string();
        let index = id
            .parse::<u32>()
            .map(CameraIndex::Index)
            .unwrap_or_else(|_| CameraIndex::String(id.clone()));
        // A single busy/unplugged camera must not fail the whole picker.
        // Recordly skips unavailable displays the same way: list the device
        // with empty formats so the UI can show it as unavailable instead of
        // reporting "no cameras".
        let formats = Camera::with_backend(
            index,
            RequestedFormat::new::<RgbAFormat>(RequestedFormatType::None),
            ApiBackend::MediaFoundation,
        )
        .ok()
        .map(|mut camera| {
            camera
                .compatible_camera_formats()
                .unwrap_or_default()
                .into_iter()
                .map(|format| CameraFormat {
                    width: format.width(),
                    height: format.height(),
                    fps: format.frame_rate(),
                })
                .collect()
        })
        .map(dedupe_and_sort_formats)
        .unwrap_or_default();
        result.push(CameraDevice {
            id,
            name: device.human_name(),
            formats,
        });
    }
    Ok(result)
}

#[cfg(not(windows))]
pub fn enumerate_devices() -> Result<Vec<CameraDevice>, CameraError> {
    Err(CameraError::Unsupported)
}

pub struct CameraRecordingHandle {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<Result<u64, CameraError>>>,
}

impl CameraRecordingHandle {
    pub fn stop(mut self) -> Result<u64, CameraError> {
        use std::sync::atomic::Ordering;
        self.stop.store(true, Ordering::Release);
        let Some(thread) = self.thread.take() else {
            return Err(CameraError::Device(
                "camera capture thread was already stopped".into(),
            ));
        };
        thread
            .join()
            .unwrap_or_else(|_| Err(CameraError::Device("camera capture thread panicked".into())))
    }
}

impl Drop for CameraRecordingHandle {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        self.stop.store(true, Ordering::Release);
        // Reap a finished thread so a disconnect-mid-capture result is not
        // lost; never block Drop on a still-running thread (that would hang
        // pause/resume). A running thread is detached and exits promptly once
        // it observes `stop`, finalizing its partial MP4.
        if let Some(thread) = self.thread.take() {
            if thread.is_finished() {
                let _ = thread.join();
            } else {
                std::mem::forget(thread);
            }
        }
    }
}

/// Preflight with an actionable message; verifies the selected camera still
/// exists before any file is created.
#[cfg(windows)]
pub fn check_device_available(device_id: &str) -> Result<(), CameraError> {
    let devices = enumerate_devices()?;
    if devices.iter().any(|d| d.id == device_id) {
        Ok(())
    } else {
        Err(CameraError::Unavailable(format!(
            "camera '{device_id}' was unplugged or is in use by another app; reconnect it or pick another camera"
        )))
    }
}

#[cfg(not(windows))]
pub fn check_device_available(_: &str) -> Result<(), CameraError> {
    Err(CameraError::Unsupported)
}

#[cfg(windows)]
pub fn start_recording(
    device_id: String,
    path: std::path::PathBuf,
) -> Result<CameraRecordingHandle, CameraError> {
    use std::sync::{Arc, atomic::AtomicBool, atomic::Ordering};
    if device_id.trim().is_empty() {
        return Err(CameraError::Unavailable(
            "no camera selected; pick a camera or turn it off".into(),
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CameraError::Device(format!("cannot create output folder: {e}")))?;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(0);
    let thread = std::thread::Builder::new()
        .name("kiri-camera".into())
        .spawn(move || capture_camera(device_id, path, thread_stop, ready_tx))
        .map_err(|error| CameraError::Device(error.to_string()))?;
    match ready_rx.recv_timeout(CAMERA_START_TIMEOUT) {
        Ok(()) => Ok(CameraRecordingHandle {
            stop,
            thread: Some(thread),
        }),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            stop.store(true, Ordering::Release);
            // Do NOT join here: the capture thread may be wedged inside
            // `Camera::with_backend` / `open_stream`, and joining would hang
            // the Tauri command the timeout was meant to protect. Detach so
            // the timeout actually returns; the thread observes `stop` and
            // exits (finalizing any partial file) once the driver unblocks.
            // The rendezvous `ready` sender is dropped with this scope, so a
            // late `ready.send(())` fails fast instead of blocking forever.
            std::mem::forget(thread);
            Err(CameraError::StartTimeout(
                "camera did not start within 12s; it may be in use by another app".into(),
            ))
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => thread
            .join()
            .unwrap_or_else(|_| Err(CameraError::Device("camera capture thread panicked".into())))
            .err()
            .map_or_else(
                || {
                    Err(CameraError::Unavailable(
                        "camera capture ended before it became ready".into(),
                    ))
                },
                Err,
            ),
    }
}

#[cfg(not(windows))]
pub fn start_recording(
    _: String,
    _: std::path::PathBuf,
) -> Result<CameraRecordingHandle, CameraError> {
    Err(CameraError::Unsupported)
}

#[cfg(windows)]
fn capture_camera(
    device_id: String,
    path: std::path::PathBuf,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ready: std::sync::mpsc::SyncSender<()>,
) -> Result<u64, CameraError> {
    use nokhwa::{
        Camera,
        pixel_format::RgbAFormat,
        utils::{ApiBackend, CameraIndex, RequestedFormat, RequestedFormatType},
    };
    use std::{sync::atomic::Ordering, time::Instant};
    use windows_capture::encoder::{
        AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
        VideoSettingsSubType,
    };

    let index = device_id
        .parse::<u32>()
        .map(CameraIndex::Index)
        .unwrap_or_else(|_| CameraIndex::String(device_id.clone()));
    let requested = RequestedFormat::new::<RgbAFormat>(RequestedFormatType::None);
    let mut camera =
        Camera::with_backend(index, requested, ApiBackend::MediaFoundation).map_err(|error| {
            CameraError::Unavailable(format!(
                "camera '{device_id}' could not be opened ({error}); it may be unplugged or in use"
            ))
        })?;
    let formats = camera
        .compatible_camera_formats()
        .map_err(|error| CameraError::Device(error.to_string()))?;
    let owned: Vec<CameraFormat> = formats
        .iter()
        .map(|format| CameraFormat {
            width: format.width(),
            height: format.height(),
            fps: format.frame_rate(),
        })
        .collect();
    if let Some(best_format) = pick_best_format(&owned)
        && let Some(native) = formats.iter().find(|format| {
            format.width() == best_format.width
                && format.height() == best_format.height
                && format.frame_rate() == best_format.fps
        })
    {
        camera
            .set_camera_requset(RequestedFormat::new::<RgbAFormat>(
                RequestedFormatType::Exact(*native),
            ))
            .map_err(|error| CameraError::Device(error.to_string()))?;
    }
    camera.open_stream().map_err(|error| {
        CameraError::Unavailable(format!(
            "camera '{device_id}' could not be started ({error}); it may be in use by another app"
        ))
    })?;
    let actual = camera.camera_format();
    let width = actual.width();
    let height = actual.height();
    validate_dimensions(width, height)?;
    // Use the camera's real frame rate instead of a hardcoded 30 so playback
    // speed stays correct on 15/60fps devices; 0 (unknown) falls back to 30.
    let fps = select_encoder_fps(actual.frame_rate());
    let mut encoder = VideoEncoder::new(
        VideoSettingsBuilder::new(width, height)
            .sub_type(VideoSettingsSubType::H264)
            .frame_rate(fps)
            .bitrate(6_000_000),
        AudioSettingsBuilder::default().disabled(true),
        ContainerSettingsBuilder::default(),
        path,
    )
    .map_err(|error| CameraError::Device(error.to_string()))?;
    let started = Instant::now();
    let _ = ready.send(());
    let mut frames = 0_u64;
    while !stop.load(Ordering::Acquire) {
        let image = match camera
            .frame()
            .and_then(|buffer| buffer.decode_image::<RgbAFormat>())
        {
            Ok(image) => image,
            Err(error) => {
                // Finalize the partial MP4 so the segment stays playable for
                // recovery, then report the disconnect clearly.
                let _ = camera.stop_stream();
                let _ = encoder.finish();
                return Err(CameraError::Unavailable(format!(
                    "camera '{device_id}' disconnected during recording ({error})"
                )));
            }
        };
        let rgba = image.into_raw();
        // A short driver buffer must finalize + report, never panic the
        // capture thread (a panic would surface as "thread panicked" with no
        // playable file and no actionable message).
        let bgra_bottom_up = match convert_rgba_to_bgra_bottom_up(&rgba, width, height) {
            Ok(buffer) => buffer,
            Err(error) => {
                let _ = camera.stop_stream();
                let _ = encoder.finish();
                return Err(CameraError::Unavailable(format!(
                    "camera '{device_id}' disconnected during recording ({error})"
                )));
            }
        };
        let timestamp_hns = hns_from_elapsed(started.elapsed());
        if let Err(error) = encoder.send_frame_buffer(&bgra_bottom_up, timestamp_hns) {
            // Finalize the partial MP4 so pause/resume + crash recovery keep
            // a playable segment instead of a corrupt 0-byte file.
            let _ = camera.stop_stream();
            let _ = encoder.finish();
            return Err(CameraError::Device(error.to_string()));
        }
        frames += 1;
    }
    camera
        .stop_stream()
        .map_err(|error| CameraError::Device(error.to_string()))?;
    encoder
        .finish()
        .map_err(|error| CameraError::Device(error.to_string()))?;
    Ok(frames)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mirror_is_preview_metadata_only() {
        assert_eq!(
            CameraMetadata::default(),
            CameraMetadata {
                mirrored_in_preview: true,
                mirror_baked_into_source: false
            }
        );
    }

    #[test]
    fn dedupe_sorts_and_removes_duplicates() {
        let out = dedupe_and_sort_formats(vec![
            CameraFormat {
                width: 1920,
                height: 1080,
                fps: 30,
            },
            CameraFormat {
                width: 640,
                height: 480,
                fps: 30,
            },
            CameraFormat {
                width: 1920,
                height: 1080,
                fps: 30,
            },
        ]);
        assert_eq!(
            out,
            vec![
                CameraFormat {
                    width: 640,
                    height: 480,
                    fps: 30,
                },
                CameraFormat {
                    width: 1920,
                    height: 1080,
                    fps: 30,
                },
            ]
        );
    }

    #[test]
    fn best_format_prefers_720p30() {
        let formats = vec![
            CameraFormat {
                width: 1920,
                height: 1080,
                fps: 60,
            },
            CameraFormat {
                width: 1280,
                height: 720,
                fps: 30,
            },
            CameraFormat {
                width: 640,
                height: 480,
                fps: 30,
            },
        ];
        assert_eq!(
            pick_best_format(&formats),
            Some(CameraFormat {
                width: 1280,
                height: 720,
                fps: 30,
            })
        );
        assert_eq!(pick_best_format(&[]), None);
    }

    #[test]
    fn best_format_scoring_weights_fps_distance() {
        // 720p60 (fps distance 30*20=600) loses to 480p30 (size distance
        // 640+240=880)? No: 600 < 880, so 720p60 wins. This pins the current
        // scoring so future changes are deliberate.
        let formats = vec![
            CameraFormat {
                width: 640,
                height: 480,
                fps: 30,
            },
            CameraFormat {
                width: 1280,
                height: 720,
                fps: 60,
            },
        ];
        assert_eq!(
            pick_best_format(&formats),
            Some(CameraFormat {
                width: 1280,
                height: 720,
                fps: 60,
            })
        );
    }

    #[test]
    fn encoder_fps_uses_device_rate_with_safe_fallback() {
        assert_eq!(select_encoder_fps(0), 30);
        assert_eq!(select_encoder_fps(15), 15);
        assert_eq!(select_encoder_fps(30), 30);
        assert_eq!(select_encoder_fps(60), 60);
        assert_eq!(select_encoder_fps(120), 60);
    }

    #[test]
    fn zero_area_dimensions_are_rejected() {
        assert!(validate_dimensions(0, 480).is_err());
        assert!(validate_dimensions(640, 0).is_err());
        assert!(validate_dimensions(0, 0).is_err());
        assert!(validate_dimensions(640, 480).is_ok());
    }

    #[test]
    fn hns_timestamps_use_100ns_units() {
        assert_eq!(
            hns_from_elapsed(std::time::Duration::from_secs(1)),
            10_000_000
        );
        assert_eq!(hns_from_elapsed(std::time::Duration::ZERO), 0);
    }

    #[test]
    fn frame_conversion_flips_vertically_and_swaps_rb() {
        // 1x2 image: top pixel red, bottom pixel blue (RGBA).
        let rgba = vec![
            255, 0, 0, 255, // top: red
            0, 0, 255, 255, // bottom: blue
        ];
        let out = convert_rgba_to_bgra_bottom_up(&rgba, 1, 2).unwrap();
        // Bottom-up BGRA: first row is the old bottom (blue -> BGRA).
        assert_eq!(out, vec![255, 0, 0, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn frame_conversion_rejects_short_buffers_instead_of_panicking() {
        let short = vec![0_u8; 3];
        assert!(convert_rgba_to_bgra_bottom_up(&short, 1, 1).is_err());
        assert!(convert_rgba_to_bgra_bottom_up(&[], 0, 0).is_err());
        assert!(convert_rgba_to_bgra_bottom_up(&[0_u8; 4], 0, 1).is_err());
    }

    #[test]
    fn empty_device_id_fails_fast_without_spawning_capture() {
        let result = start_recording(
            "   ".into(),
            std::path::PathBuf::from("target/test-camera-empty.mp4"),
        );
        // Non-Windows builds always report Unsupported; Windows reports the
        // actionable Unavailable message. Either way it must fail
        // synchronously (no 12s hang, no file created).
        assert!(result.is_err());
        #[cfg(windows)]
        assert!(matches!(result, Err(CameraError::Unavailable(_))));
        #[cfg(not(windows))]
        assert!(matches!(result, Err(CameraError::Unsupported)));
    }

    #[test]
    fn rapid_empty_starts_all_fail_fast() {
        // Rapid start/stop cycles with no camera selected must never hang,
        // leak threads, or create files.
        for _ in 0..50 {
            let result = start_recording(
                String::new(),
                std::path::PathBuf::from("target/test-camera-rapid.mp4"),
            );
            assert!(result.is_err());
        }
    }

    #[test]
    fn double_stop_reports_already_stopped() {
        // `stop(self)` consumes the handle, so a second stop surfaces as the
        // explicit "already stopped" error instead of joining a dead thread.
        let handle = CameraRecordingHandle {
            stop: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            thread: None,
        };
        let result = handle.stop();
        assert!(matches!(result, Err(CameraError::Device(_))));
    }

    #[test]
    fn format_helpers_never_panic_on_extreme_inputs() {
        // Property-style: arbitrary dimensions/fps never panic.
        for (w, h, fps) in [(0, 0, 0), (u32::MAX, u32::MAX, u32::MAX), (1, 1, 1)] {
            let formats = vec![CameraFormat {
                width: w,
                height: h,
                fps,
            }];
            let _ = pick_best_format(&formats);
            let _ = select_encoder_fps(fps);
            let _ = dedupe_and_sort_formats(formats);
        }
        assert_eq!(select_encoder_fps(u32::MAX), 60);
    }
}
