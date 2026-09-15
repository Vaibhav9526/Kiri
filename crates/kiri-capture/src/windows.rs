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
    RectI32 {
        left: value.left,
        top: value.top,
        width: value.right - value.left,
        height: value.bottom - value.top,
    }
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
            dpi: unsafe { GetDpiForSystem() },
            availability: SourceAvailability::Available,
            thumbnail_data_url: None,
        });
    }

    for window in Window::enumerate().map_err(|e| CaptureError::Native(e.to_string()))? {
        let raw = window.as_raw_hwnd();
        let hwnd = HWND(raw);
        let availability = if !unsafe { IsWindow(Some(hwnd)).as_bool() } {
            SourceAvailability::Closed
        } else if unsafe { IsIconic(hwnd).as_bool() } {
            SourceAvailability::Minimized
        } else if window.is_valid() {
            SourceAvailability::Available
        } else {
            SourceAvailability::Invalid
        };
        let bounds = window.rect().map(rect).unwrap_or(RectI32 {
            left: 0,
            top: 0,
            width: 0,
            height: 0,
        });
        result.push(CaptureSource {
            id: format!("window:{}", raw as usize),
            kind: CaptureSourceKind::Window,
            title: window.title().unwrap_or_default(),
            process_name: window.process_name().ok(),
            bounds,
            dpi: unsafe { GetDpiForWindow(hwnd) },
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
            let encoder = VideoEncoder::new(
                VideoSettingsBuilder::new(ctx.flags.0, ctx.flags.1)
                    .sub_type(VideoSettingsSubType::H264)
                    .frame_rate(ctx.flags.3)
                    .bitrate(ctx.flags.4),
                AudioSettingsBuilder::default().disabled(true),
                ContainerSettingsBuilder::default(),
                &ctx.flags.2,
            )?;
            Ok(Self {
                encoder: Some(encoder),
                metrics: ctx.flags.5,
                last_frame: None,
                minimum_interval_micros: 1_000_000 / u64::from(ctx.flags.3),
            })
        }

        fn on_frame_arrived(
            &mut self,
            frame: &mut Frame,
            _: InternalCaptureControl,
        ) -> Result<(), Self::Error> {
            let now = Instant::now();
            if self.last_frame.is_some_and(|last| {
                now.duration_since(last).as_micros()
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
        control: CaptureControl<ScreenHandler, NativeError>,
        metrics: Arc<ScreenMetrics>,
        started: Instant,
    }

    impl ScreenRecordingHandle {
        pub fn stop(self) -> Result<(u64, u64, f64), CaptureError> {
            let result = (
                self.metrics.encoded.load(Ordering::Relaxed),
                self.metrics.dropped.load(Ordering::Relaxed),
                self.started.elapsed().as_secs_f64(),
            );
            self.control
                .stop()
                .map_err(|e| CaptureError::Native(e.to_string()))?;
            Ok(result)
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
                .find(|monitor| monitor.device_name().ok().as_deref() == Some(value))
                .ok_or_else(|| {
                    CaptureError::SourceUnavailable(format!(
                        "selected display is no longer available ({}); pick another display",
                        config.source_id
                    ))
                })?;
            let flags = (
                monitor
                    .width()
                    .map_err(|e| CaptureError::Native(e.to_string()))?,
                monitor
                    .height()
                    .map_err(|e| CaptureError::Native(e.to_string()))?,
                config.output,
                config.fps,
                config.bitrate,
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
            let width = u32::try_from(rectangle.right - rectangle.left).map_err(|_| {
                CaptureError::SourceUnavailable(
                    "selected window has no visible area; restore it and try again".into(),
                )
            })?;
            let height = u32::try_from(rectangle.bottom - rectangle.top).map_err(|_| {
                CaptureError::SourceUnavailable(
                    "selected window has no visible area; restore it and try again".into(),
                )
            })?;
            if width == 0 || height == 0 {
                return Err(CaptureError::SourceUnavailable(
                    "selected window has no visible area; restore it and try again".into(),
                ));
            }
            let flags = (
                width,
                height,
                config.output,
                config.fps,
                config.bitrate,
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
            control,
            metrics,
            started: Instant::now(),
        })
    }
}

pub use session::{ScreenRecordingConfig, ScreenRecordingHandle, start_screen_segment};

mod thumbnail {
    use crate::CaptureError;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::{error::Error, path::PathBuf};
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
    pub fn capture(source_id: &str) -> Result<String, CaptureError> {
        let temp = tempfile::Builder::new()
            .suffix(".jpg")
            .tempfile()
            .map_err(CaptureError::Io)?;
        let path = temp.path().to_path_buf();
        if let Some(value) = source_id.strip_prefix("display:") {
            let item = Monitor::enumerate()
                .map_err(|e| CaptureError::Native(e.to_string()))?
                .into_iter()
                .find(|m| m.device_name().ok().as_deref() == Some(value))
                .ok_or_else(|| CaptureError::SourceUnavailable(source_id.into()))?;
            Handler::start(settings(item, path.clone()))
                .map_err(|e| CaptureError::Native(e.to_string()))?;
        } else if let Some(value) = source_id.strip_prefix("window:") {
            let raw = value
                .parse::<usize>()
                .map_err(|_| CaptureError::InvalidConfig("invalid window ID".into()))?
                as *mut std::ffi::c_void;
            let item = Window::from_raw_hwnd(raw);
            if !item.is_valid() {
                return Err(CaptureError::SourceUnavailable(source_id.into()));
            }
            Handler::start(settings(item, path.clone()))
                .map_err(|e| CaptureError::Native(e.to_string()))?;
        } else {
            return Err(CaptureError::InvalidConfig("unknown source ID".into()));
        }
        let bytes = std::fs::read(path).map_err(CaptureError::Io)?;
        Ok(format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes)))
    }
}
pub use thumbnail::capture as capture_thumbnail;
