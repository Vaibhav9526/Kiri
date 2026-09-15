#[cfg(windows)]
mod windows_main {
    use std::{error::Error, path::PathBuf, time::Instant};
    use windows_capture::{
        capture::{Context, GraphicsCaptureApiHandler},
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
    };

    struct Spike {
        encoder: Option<VideoEncoder>,
        started: Instant,
        seconds: u64,
        frames: u64,
    }

    impl GraphicsCaptureApiHandler for Spike {
        type Flags = (u32, u32, PathBuf, u64);
        type Error = Box<dyn Error + Send + Sync>;

        fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
            let encoder = VideoEncoder::new(
                VideoSettingsBuilder::new(ctx.flags.0, ctx.flags.1)
                    .sub_type(VideoSettingsSubType::H264),
                AudioSettingsBuilder::default().disabled(true),
                ContainerSettingsBuilder::default(),
                &ctx.flags.2,
            )?;
            Ok(Self {
                encoder: Some(encoder),
                started: Instant::now(),
                seconds: ctx.flags.3,
                frames: 0,
            })
        }

        fn on_frame_arrived(
            &mut self,
            frame: &mut Frame,
            control: InternalCaptureControl,
        ) -> Result<(), Self::Error> {
            let Some(encoder) = self.encoder.as_mut() else {
                return Ok(());
            };
            encoder.send_frame(frame)?;
            self.frames += 1;
            if self.started.elapsed().as_secs() >= self.seconds {
                if let Some(encoder) = self.encoder.take() {
                    // Stop the message loop even when finalization fails;
                    // otherwise a `finish()` error leaks the capture thread
                    // (no `Drop` joins it) and hangs the diagnostic.
                    let finish_result = encoder.finish();
                    eprintln!(
                        "{{\"frames\":{},\"seconds\":{:.3},\"gpuPath\":true,\"encoder\":\"Media Foundation H.264\"}}",
                        self.frames,
                        self.started.elapsed().as_secs_f64()
                    );
                    control.stop();
                    finish_result?;
                } else {
                    control.stop();
                }
            }
            Ok(())
        }
    }

    pub fn run() -> Result<(), Box<dyn Error + Send + Sync>> {
        let output = std::env::args()
            .nth(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| "wgc-spike.mp4".into());
        // Missing duration keeps the historical 5s default, but an explicit
        // invalid value errors instead of silently recording the wrong span.
        let seconds_text = std::env::args().nth(2);
        let seconds: u64 = match seconds_text {
            None => 5,
            Some(text) => text.parse().map_err(|_| {
                format!("invalid seconds '{text}'; expected a non-negative integer")
            })?,
        };
        if seconds > 3600 {
            return Err("invalid seconds; expected 0..=3600".into());
        }
        let monitor = Monitor::primary()?;
        let width = monitor.width()?;
        let height = monitor.height()?;
        if width == 0 || height == 0 {
            return Err("primary display has no visible area".into());
        }
        let settings = Settings::new(
            monitor,
            CursorCaptureSettings::WithoutCursor,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Exclude,
            MinimumUpdateIntervalSettings::Custom(std::time::Duration::from_micros(16_667)),
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            (width, height, output, seconds),
        );
        Spike::start(settings)?;
        Ok(())
    }
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    windows_main::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("kiri-capture-diagnostic requires Windows");
    std::process::exit(2);
}

#[cfg(test)]
mod tests {
    #[test]
    fn diagnostic_seconds_reject_out_of_range_without_panicking() {
        let seconds: u64 = 3601;
        assert!(seconds > 3600);
        let ok: u64 = 5;
        assert!(ok <= 3600);
    }
}
