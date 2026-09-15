use crate::{CaptureError, CaptureSource, CaptureSourceKind, RectI32, SourceAvailability};
use ::windows::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO},
    UI::{
        HiDpi::{GetDpiForSystem, GetDpiForWindow},
        WindowsAndMessaging::{IsIconic, IsWindow},
    },
};
use windows_capture::{monitor::Monitor, window::Window};
// capture session implementation follows source enumeration.

fn rect(value: RECT) -> RectI32 {
    // `right - left` can overflow `i32` for hostile rects; widen first and
    // clamp so a corrupt window rect never panics enumeration in debug.
    let width = (i64::from(value.right) - i64::from(value.left))
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    let height = (i64::from(value.bottom) - i64::from(value.top))
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    RectI32 {
        left: value.left,
        top: value.top,
        width,
        height,
    }
}

/// `GetDpiForWindow`/`GetDpiForSystem` return 0 when the handle is dead; fall
/// back to 96 (100%) so downstream DPI math never collapses to the origin.
fn effective_dpi(raw: u32, fallback: u32) -> u32 {
    if raw == 0 { fallback.max(96) } else { raw }
}

pub fn enumerate_sources() -> Result<Vec<CaptureSource>, CaptureError> {
    let mut result = Vec::new();
    for monitor in Monitor::enumerate().map_err(|e| CaptureError::Native(e.to_string()))? {
        let raw = monitor.as_raw_hmonitor();
        let hmonitor = HMONITOR(raw);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        // One bad monitor must not fail the whole picker; skip it like
        // Recordly skips unavailable displays during availability checks.
        if !unsafe { GetMonitorInfoW(hmonitor, &mut info) }.as_bool() {
            continue;
        }
        result.push(CaptureSource {
            id: format!(
                "display:{}",
                monitor
                    .device_name()
                    .unwrap_or_else(|_| format!("{}", raw as usize))
            ),
            kind: CaptureSourceKind::Display,
            title: monitor.name().unwrap_or_else(|_| "Display".into()),
            process_name: None,
            bounds: rect(info.rcMonitor),
            dpi: effective_dpi(unsafe { GetDpiForSystem() }, 96),
            availability: SourceAvailability::Available,
            thumbnail_data_url: None,
        });
    }

    for window in Window::enumerate().map_err(|e| CaptureError::Native(e.to_string()))? {
        let raw = window.as_raw_hwnd();
        let hwnd = HWND(raw);
        let bounds = window.rect().map(rect).unwrap_or(RectI32 {
            left: 0,
            top: 0,
            width: 0,
            height: 0,
        });
        // A zero-area window cannot produce frames; report it as Invalid so
        // the picker never offers it as Available (mirrors Recordly's
        // "no visible area" guard at segment start).
        let availability = if !unsafe { IsWindow(Some(hwnd)).as_bool() } {
            SourceAvailability::Closed
        } else if unsafe { IsIconic(hwnd).as_bool() } {
            SourceAvailability::Minimized
        } else if bounds.width <= 0 || bounds.height <= 0 {
            SourceAvailability::Invalid
        } else if window.is_valid() {
            SourceAvailability::Available
        } else {
            SourceAvailability::Invalid
        };
        // `GetDpiForWindow` returns 0 for a dead handle; fall back to the
        // system DPI so downstream coordinate math stays sane.
        let system_dpi = effective_dpi(unsafe { GetDpiForSystem() }, 96);
        let dpi = effective_dpi(unsafe { GetDpiForWindow(hwnd) }, system_dpi);
        result.push(CaptureSource {
            id: format!("window:{}", raw as usize),
            kind: CaptureSourceKind::Window,
            title: window.title().unwrap_or_default(),
            process_name: window.process_name().ok(),
            bounds,
            dpi,
            availability,
            thumbnail_data_url: None,
        });
    }
    Ok(result)
}

/// Friendly preflight for a capture source, mirroring Recordly's
/// availability phase. Returns the live source or a clear,
/// user-actionable error for missing/minimized/closed/protected windows.
pub fn validate_source_for_capture(source_id: &str) -> Result<CaptureSource, CaptureError> {
    let enumerated = enumerate_sources()?;
    let source = enumerated
        .into_iter()
        .find(|source| source.id == source_id)
        .ok_or_else(|| {
            CaptureError::SourceUnavailable(format!(
                "selected source is no longer available ({source_id}); pick another display or window"
            ))
        })?;
    match source.availability {
        SourceAvailability::Available => Ok(source),
        SourceAvailability::Minimized => Err(CaptureError::SourceUnavailable(format!(
            "selected window '{}' is minimized; restore it and try again",
            source.title
        ))),
        SourceAvailability::Closed => Err(CaptureError::SourceUnavailable(format!(
            "selected window '{}' was closed before recording started",
            source.title
        ))),
        SourceAvailability::Protected => Err(CaptureError::SourceUnavailable(format!(
            "selected window '{}' is protected and cannot be captured; pick another source",
            source.title
        ))),
        SourceAvailability::Invalid => Err(CaptureError::SourceUnavailable(format!(
            "selected source '{}' is not currently capturable",
            source.title
        ))),
    }
}

mod session {
    use crate::CaptureError;
    use std::{
        error::Error,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        },
        time::Instant,
    };
    use windows_capture::{
        capture::{CaptureControl, Context, GraphicsCaptureApiHandler},
        encoder::{
            AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
            VideoSettingsSubType,
        },
        frame::Frame,
        graphics_capture_api::InternalCaptureControl,
        monitor::Monitor,
        settings::{
            ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
            MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
        },
        window::Window,
    };
    type NativeError = Box<dyn Error + Send + Sync>;

    #[derive(Clone, Debug)]
    pub struct ScreenRecordingConfig {
        pub source_id: String,
        pub output: PathBuf,
        pub fps: u32,
        pub bitrate: u32,
    }

    #[derive(Default)]
    struct ScreenMetrics {
        encoded: AtomicU64,
        dropped: AtomicU64,
    }

    struct ScreenHandler {
        encoder: Option<VideoEncoder>,
        metrics: Arc<ScreenMetrics>,
        last_frame: Option<Instant>,
        minimum_interval_micros: u64,
    }

    impl GraphicsCaptureApiHandler for ScreenHandler {
        type Flags = (u32, u32, PathBuf, u32, u32, Arc<ScreenMetrics>);
        type Error = NativeError;

        fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
            // `start_screen_segment` validates 30|60 FPS, but never divide by
            // a runtime FPS without a guard: a zero would panic the capture
            // thread instead of failing the segment cleanly.
            let fps = ctx.flags.3.max(1);
            let encoder = VideoEncoder::new(
                VideoSettingsBuilder::new(ctx.flags.0, ctx.flags.1)
                    .sub_type(VideoSettingsSubType::H264)
                    .frame_rate(fps)
                    .bitrate(ctx.flags.4),
                AudioSettingsBuilder::default().disabled(true),
                ContainerSettingsBuilder::default(),
                &ctx.flags.2,
            )?;
            Ok(Self {
                encoder: Some(encoder),
                metrics: ctx.flags.5,
                last_frame: None,
                minimum_interval_micros: 1_000_000 / u64::from(fps),
            })
        }

        fn on_frame_arrived(
            &mut self,
            frame: &mut Frame,
            _: InternalCaptureControl,
        ) -> Result<(), Self::Error> {
            let now = Instant::now();
            if self.last_frame.is_some_and(|last| {
                now.saturating_duration_since(last).as_micros()
                    < u128::from(self.minimum_interval_micros * 7 / 10)
            }) {
                self.metrics.dropped.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
            let Some(encoder) = self.encoder.as_mut() else {
                return Ok(());
            };
            encoder.send_frame(frame)?;
            self.last_frame = Some(now);
            self.metrics.encoded.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    pub struct ScreenRecordingHandle {
        control: Option<CaptureControl<ScreenHandler, NativeError>>,
        metrics: Arc<ScreenMetrics>,
        started: Instant,
    }

    impl ScreenRecordingHandle {
        pub fn stop(mut self) -> Result<(u64, u64, f64), CaptureError> {
            let result = (
                self.metrics.encoded.load(Ordering::Relaxed),
                self.metrics.dropped.load(Ordering::Relaxed),
                self.started.elapsed().as_secs_f64(),
            );
            let Some(control) = self.control.take() else {
                return Err(CaptureError::Native(
                    "capture session was already stopped".into(),
                ));
            };
            control
                .stop()
                .map_err(|e| CaptureError::Native(e.to_string()))?;
            Ok(result)
        }
    }

    impl Drop for ScreenRecordingHandle {
        fn drop(&mut self) {
            // `CaptureControl` has no `Drop`: dropping it without `stop()`
            // detaches the capture thread and leaves the MP4 unfinalized
            // (zero-byte output). Best-effort stop here so a panic or early
            // return never leaks the capture thread; explicit `stop()` takes
            // the control first so this is a no-op on the clean path.
            if let Some(control) = self.control.take() {
                let _ = control.stop();
            }
        }
    }

    pub fn start_screen_segment(
        config: ScreenRecordingConfig,
    ) -> Result<ScreenRecordingHandle, CaptureError> {
        if !matches!(config.fps, 30 | 60) {
            return Err(CaptureError::InvalidConfig(
                "FPS must be 30 or 60; check recording settings".into(),
            ));
        }
        if config.source_id.trim().is_empty() {
            return Err(CaptureError::InvalidConfig(
                "no capture source selected; pick a display or window".into(),
            ));
        }
        // Fail fast with a friendly message before allocating the encoder or
        // touching the filesystem, mirroring Recordly's availability phase.
        crate::windows::validate_source_for_capture(&config.source_id)?;
        if let Some(parent) = config.output.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                CaptureError::Io(std::io::Error::new(
                    e.kind(),
                    format!("cannot create output folder: {e}"),
                ))
            })?;
        }
        let metrics = Arc::new(ScreenMetrics::default());
        let interval = MinimumUpdateIntervalSettings::Custom(std::time::Duration::from_micros(
            1_000_000 / u64::from(config.fps),
        ));
        let control = if let Some(value) = config.source_id.strip_prefix("display:") {
            let monitor = Monitor::enumerate()
                .map_err(|e| CaptureError::Native(e.to_string()))?
                .into_iter()
                .find(|monitor| {
                    // Enumeration falls back to the raw HMONITOR value when
                    // `device_name()` fails; match that fallback too so every
                    // enumerated `display:*` ID round-trips to a segment.
                    if monitor.device_name().ok().as_deref() == Some(value) {
                        return true;
                    }
                    format!("{}", monitor.as_raw_hmonitor() as usize) == value
                })
                .ok_or_else(|| {
                    CaptureError::SourceUnavailable(format!(
                        "selected display is no longer available ({}); pick another display",
                        config.source_id
                    ))
                })?;
            let width = monitor
                .width()
                .map_err(|e| CaptureError::Native(e.to_string()))?;
            let height = monitor
                .height()
                .map_err(|e| CaptureError::Native(e.to_string()))?;
            if width == 0 || height == 0 {
                return Err(CaptureError::SourceUnavailable(
                    "selected display has no visible area; pick another display".into(),
                ));
            }
            let bitrate = if config.bitrate == 0 {
                crate::default_video_bitrate(width, height, config.fps)
            } else {
                config.bitrate
            };
            let flags = (
                width,
                height,
                config.output,
                config.fps,
                bitrate,
                Arc::clone(&metrics),
            );
            ScreenHandler::start_free_threaded(Settings::new(
                monitor,
                CursorCaptureSettings::WithoutCursor,
                DrawBorderSettings::WithoutBorder,
                SecondaryWindowSettings::Exclude,
                interval,
                DirtyRegionSettings::Default,
                ColorFormat::Bgra8,
                flags,
            ))
            .map_err(|e| CaptureError::Native(e.to_string()))?
        } else if let Some(value) = config.source_id.strip_prefix("window:") {
            let raw = value.parse::<usize>().map_err(|_| {
                CaptureError::InvalidConfig("invalid window ID; reselect the window".into())
            })? as *mut std::ffi::c_void;
            let window = Window::from_raw_hwnd(raw);
            if !window.is_valid() {
                return Err(CaptureError::SourceUnavailable(
                    "selected window is no longer available; it may have been closed".into(),
                ));
            }
            let rectangle = window
                .rect()
                .map_err(|e| CaptureError::Native(e.to_string()))?;
            // `right - left` in `i32` can overflow in debug for hostile
            // rects; widen to `i64` first, then range-check.
            let width_i64 = i64::from(rectangle.right) - i64::from(rectangle.left);
            let height_i64 = i64::from(rectangle.bottom) - i64::from(rectangle.top);
            if !(1..=i64::from(u32::MAX)).contains(&width_i64)
                || !(1..=i64::from(u32::MAX)).contains(&height_i64)
            {
                return Err(CaptureError::SourceUnavailable(
                    "selected window has no visible area; restore it and try again".into(),
                ));
            }
            let width = width_i64 as u32;
            let height = height_i64 as u32;
            let bitrate = if config.bitrate == 0 {
                crate::default_video_bitrate(width, height, config.fps)
            } else {
                config.bitrate
            };
            let flags = (
                width,
                height,
                config.output,
                config.fps,
                bitrate,
                Arc::clone(&metrics),
            );
            ScreenHandler::start_free_threaded(Settings::new(
                window,
                CursorCaptureSettings::WithoutCursor,
                DrawBorderSettings::WithoutBorder,
                SecondaryWindowSettings::Exclude,
                interval,
                DirtyRegionSettings::Default,
                ColorFormat::Bgra8,
                flags,
            ))
            .map_err(|e| CaptureError::Native(e.to_string()))?
        } else {
            return Err(CaptureError::InvalidConfig(
                "unknown capture source ID; reselect the source".into(),
            ));
        };
        Ok(ScreenRecordingHandle {
            control: Some(control),
            metrics,
            started: Instant::now(),
        })
    }
}

pub use session::{ScreenRecordingConfig, ScreenRecordingHandle, start_screen_segment};

mod thumbnail {
    use crate::CaptureError;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::{
        error::Error,
        path::PathBuf,
        time::{Duration, Instant},
    };
    use windows_capture::{
        capture::{Context, GraphicsCaptureApiHandler},
        encoder::ImageFormat,
        frame::Frame,
        graphics_capture_api::InternalCaptureControl,
        monitor::Monitor,
        settings::{
            ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
            MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
        },
        window::Window,
    };
    struct Handler {
        path: PathBuf,
    }
    impl GraphicsCaptureApiHandler for Handler {
        type Flags = PathBuf;
        type Error = Box<dyn Error + Send + Sync>;
        fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
            Ok(Self { path: ctx.flags })
        }
        fn on_frame_arrived(
            &mut self,
            frame: &mut Frame,
            control: InternalCaptureControl,
        ) -> Result<(), Self::Error> {
            frame.save_as_image(&self.path, ImageFormat::Jpeg)?;
            control.stop();
            Ok(())
        }
    }
    fn settings<T>(item: T, path: PathBuf) -> Settings<PathBuf, T>
    where
        T: TryInto<windows_capture::settings::GraphicsCaptureItemType>,
    {
        Settings::new(
            item,
            CursorCaptureSettings::WithoutCursor,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Exclude,
            MinimumUpdateIntervalSettings::Default,
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            path,
        )
    }
    /// Best-effort single-frame thumbnail with a timeout. A minimized,
    /// occluded or protected source may never deliver a frame; blocking
    /// forever would hang the source picker, so this mirrors Recordly's
    /// start-timeout behavior and reports a timeout as `SourceUnavailable`.
    pub fn capture(source_id: &str) -> Result<String, CaptureError> {
        // Use a temp dir + not-yet-existing path: a pre-created `NamedTempFile`
        // keeps an open handle that `save_as_image` cannot overwrite on
        // Windows (sharing violation), failing every thumbnail.
        let dir = tempfile::tempdir().map_err(CaptureError::Io)?;
        let path = dir.path().join("thumbnail.jpg");
        let control = if let Some(value) = source_id.strip_prefix("display:") {
            let item = Monitor::enumerate()
                .map_err(|e| CaptureError::Native(e.to_string()))?
                .into_iter()
                .find(|m| {
                    if m.device_name().ok().as_deref() == Some(value) {
                        return true;
                    }
                    format!("{}", m.as_raw_hmonitor() as usize) == value
                })
                .ok_or_else(|| CaptureError::SourceUnavailable(source_id.into()))?;
            Handler::start_free_threaded(settings(item, path.clone()))
                .map_err(|e| CaptureError::Native(e.to_string()))?
        } else if let Some(value) = source_id.strip_prefix("window:") {
            let raw = value
                .parse::<usize>()
                .map_err(|_| CaptureError::InvalidConfig("invalid window ID".into()))?
                as *mut std::ffi::c_void;
            let item = Window::from_raw_hwnd(raw);
            if !item.is_valid() {
                return Err(CaptureError::SourceUnavailable(source_id.into()));
            }
            Handler::start_free_threaded(settings(item, path.clone()))
                .map_err(|e| CaptureError::Native(e.to_string()))?
        } else {
            return Err(CaptureError::InvalidConfig("unknown source ID".into()));
        };
        // Wait for the first frame to land; `control.stop()` below always
        // joins the capture thread so a timeout never leaks it.
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut ready = false;
        while Instant::now() < deadline {
            match std::fs::metadata(&path) {
                Ok(meta) if meta.is_file() && meta.len() > 0 => {
                    ready = true;
                    break;
                }
                _ => {}
            }
            if control.is_finished() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        // Always join the capture thread, even on timeout, to avoid leaking
        // the free-threaded worker (which has no `Drop`).
        let stop_result = control.stop();
        if !ready {
            // Surface a real encoder failure if the thread died with one;
            // otherwise report the timeout as an unavailable source.
            if let Err(e) = stop_result {
                return Err(CaptureError::Native(e.to_string()));
            }
            return Err(CaptureError::SourceUnavailable(format!(
                "thumbnail for '{source_id}' timed out; the source may be minimized or protected"
            )));
        }
        // The frame is on disk; a failed join now must not hide it, but an
        // encoder error with no file still surfaces below via the size check.
        let _ = stop_result;
        let bytes = std::fs::read(&path).map_err(CaptureError::Io)?;
        if bytes.is_empty() {
            return Err(CaptureError::Native(
                "thumbnail capture produced an empty image".into(),
            ));
        }
        Ok(format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes)))
    }
}
pub use thumbnail::capture as capture_thumbnail;

#[cfg(test)]
mod tests {
    use super::{effective_dpi, rect};
    use ::windows::Win32::Foundation::RECT;

    #[test]
    fn rect_conversion_never_panics_on_hostile_input() {
        let hostile = RECT {
            left: i32::MIN,
            top: i32::MIN,
            right: i32::MAX,
            bottom: i32::MAX,
        };
        let converted = rect(hostile);
        assert_eq!(converted.left, i32::MIN);
        assert_eq!(converted.width, i32::MAX);

        let inverted = RECT {
            left: 100,
            top: 100,
            right: 50,
            bottom: 40,
        };
        let converted = rect(inverted);
        assert_eq!(converted.width, -50);
        assert_eq!(converted.height, -60);
    }

    #[test]
    fn dpi_fallback_keeps_dead_handles_sane() {
        assert_eq!(effective_dpi(0, 96), 96);
        assert_eq!(effective_dpi(0, 0), 96);
        assert_eq!(effective_dpi(144, 96), 144);
        // Downstream math with the fallback matches 100% scaling.
        let bounds = crate::RectI32 {
            left: 10,
            top: 20,
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            bounds.physical_from_logical(100.0, 80.0, effective_dpi(0, 96)),
            (110, 100)
        );
    }

    #[test]
    fn zero_bitrate_config_falls_back_to_recordly_tiers() {
        assert_eq!(crate::default_video_bitrate(1920, 1080, 30), 18_000_000);
        assert_eq!(crate::default_video_bitrate(1920, 1080, 60), 24_300_000);
    }

    #[test]
    fn window_extent_math_rejects_empty_area_without_panicking() {
        // Mirrors `start_screen_segment`: hostile RECT differences widen to
        // `i64` before the range check, so debug builds never overflow.
        let width_i64 = i64::from(i32::MAX) - i64::from(i32::MIN);
        assert!(!(1..=i64::from(u32::MAX)).contains(&width_i64));
        let width_i64 = i64::from(1920) - i64::from(0);
        assert!((1..=i64::from(u32::MAX)).contains(&width_i64));
    }
}
