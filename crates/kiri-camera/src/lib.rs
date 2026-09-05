use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CameraError {
    #[error("camera capture is only available on Windows")]
    Unsupported,
    #[error("camera device failed: {0}")]
    Device(String),
}

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
        let mut camera = Camera::with_backend(
            index,
            RequestedFormat::new::<RgbAFormat>(RequestedFormatType::None),
            ApiBackend::MediaFoundation,
        )
        .map_err(|e| CameraError::Device(e.to_string()))?;
        let mut formats: Vec<CameraFormat> = camera
            .compatible_camera_formats()
            .unwrap_or_default()
            .into_iter()
            .map(|format| CameraFormat {
                width: format.width(),
                height: format.height(),
                fps: format.frame_rate(),
            })
            .collect();
        formats.sort_by_key(|format| (format.width, format.height, format.fps));
        formats.dedup();
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
        self.thread
            .take()
            .expect("camera thread exists")
            .join()
            .map_err(|_| CameraError::Device("camera capture thread panicked".into()))?
    }
}

#[cfg(windows)]
pub fn start_recording(
    device_id: String,
    path: std::path::PathBuf,
) -> Result<CameraRecordingHandle, CameraError> {
    use std::sync::{Arc, atomic::AtomicBool};
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(0);
    let thread = std::thread::Builder::new()
        .name("kiri-camera".into())
        .spawn(move || capture_camera(device_id, path, thread_stop, ready_tx))
        .map_err(|error| CameraError::Device(error.to_string()))?;
    if ready_rx.recv().is_err() {
        return Err(thread
            .join()
            .map_err(|_| CameraError::Device("camera capture thread panicked".into()))?
            .err()
            .unwrap_or_else(|| CameraError::Device("camera capture ended before ready".into())));
    }
    Ok(CameraRecordingHandle {
        stop,
        thread: Some(thread),
    })
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
        .unwrap_or_else(|_| CameraIndex::String(device_id));
    let requested = RequestedFormat::new::<RgbAFormat>(RequestedFormatType::None);
    let mut camera = Camera::with_backend(index, requested, ApiBackend::MediaFoundation)
        .map_err(|error| CameraError::Device(error.to_string()))?;
    let formats = camera
        .compatible_camera_formats()
        .map_err(|error| CameraError::Device(error.to_string()))?;
    if let Some(best) = formats.iter().min_by_key(|format| {
        format.width().abs_diff(1280)
            + format.height().abs_diff(720)
            + format.frame_rate().abs_diff(30) * 20
    }) {
        camera
            .set_camera_requset(RequestedFormat::new::<RgbAFormat>(
                RequestedFormatType::Exact(*best),
            ))
            .map_err(|error| CameraError::Device(error.to_string()))?;
    }
    camera
        .open_stream()
        .map_err(|error| CameraError::Device(error.to_string()))?;
    let actual = camera.camera_format();
    let width = actual.width();
    let height = actual.height();
    let fps = 30;
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
        let image = camera
            .frame()
            .and_then(|buffer| buffer.decode_image::<RgbAFormat>())
            .map_err(|error| CameraError::Device(error.to_string()))?;
        let rgba = image.into_raw();
        let row_bytes = width as usize * 4;
        let mut bgra_bottom_up = vec![0_u8; rgba.len()];
        for y in 0..height as usize {
            let source = &rgba[y * row_bytes..(y + 1) * row_bytes];
            let destination_y = height as usize - 1 - y;
            let destination =
                &mut bgra_bottom_up[destination_y * row_bytes..(destination_y + 1) * row_bytes];
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
        let timestamp_hns = (started.elapsed().as_nanos() / 100).min(i64::MAX as u128) as i64;
        encoder
            .send_frame_buffer(&bgra_bottom_up, timestamp_hns)
            .map_err(|error| CameraError::Device(error.to_string()))?;
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
}
