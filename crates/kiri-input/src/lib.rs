use kiri_capture::{NormalizedPoint, RectI32};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const TELEMETRY_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    X1,
    X2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ButtonPhase {
    Press,
    Release,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorSample {
    pub timestamp_micros: i64,
    pub screen_x: i32,
    pub screen_y: i32,
    pub normalized: NormalizedPoint,
}

impl CursorSample {
    #[must_use]
    pub fn map(timestamp_micros: i64, screen_x: i32, screen_y: i32, source: RectI32) -> Self {
        Self {
            timestamp_micros,
            screen_x,
            screen_y,
            normalized: source.normalize(screen_x, screen_y),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MouseEvent {
    pub timestamp_micros: i64,
    pub screen_x: i32,
    pub screen_y: i32,
    pub button: Option<MouseButton>,
    pub phase: Option<ButtonPhase>,
    pub wheel_delta: i16,
    pub click_count: u8,
}

#[derive(Debug, Error)]
pub enum InputError {
    #[error("input telemetry is only available on Windows")]
    Unsupported,
    #[error("input telemetry failed: {0}")]
    Native(String),
}

#[cfg(windows)]
mod recorder {
    use super::*;
    use std::{
        fs::{self, OpenOptions},
        io::{BufWriter, Write},
        path::PathBuf,
        sync::{
            Mutex, OnceLock,
            mpsc::{Receiver, SyncSender, sync_channel},
        },
        thread::JoinHandle,
        time::Instant,
    };
    use windows::Win32::{
        Foundation::{LPARAM, LRESULT, WPARAM},
        System::Threading::GetCurrentThreadId,
        UI::WindowsAndMessaging::{
            CallNextHookEx, GetMessageW, HHOOK, MSG, MSLLHOOKSTRUCT, PostThreadMessageW,
            SetWindowsHookExW, UnhookWindowsHookEx, WH_MOUSE_LL, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
            WM_LBUTTONUP, WM_MBUTTONDBLCLK, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE,
            WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDBLCLK, WM_RBUTTONDOWN, WM_RBUTTONUP,
            WM_XBUTTONDBLCLK, WM_XBUTTONDOWN, WM_XBUTTONUP,
        },
    };

    #[derive(Clone, Copy)]
    struct HookContext {
        source: RectI32,
        offset_micros: i64,
        started: Instant,
    }

    #[derive(Clone, Copy)]
    enum HookEvent {
        Cursor(CursorSample),
        Mouse(MouseEvent),
    }

    static SENDER: OnceLock<Mutex<Option<SyncSender<HookEvent>>>> = OnceLock::new();
    static CONTEXT: OnceLock<Mutex<Option<HookContext>>> = OnceLock::new();

    unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let data = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
            let context = CONTEXT
                .get()
                .and_then(|value| value.lock().ok())
                .and_then(|value| *value);
            if let Some(context) = context {
                let timestamp_micros = context.offset_micros
                    + context.started.elapsed().as_micros().min(i64::MAX as u128) as i64;
                let message = wparam.0 as u32;
                let cursor =
                    CursorSample::map(timestamp_micros, data.pt.x, data.pt.y, context.source);
                if message == WM_MOUSEMOVE {
                    if let Some(sender) = SENDER.get().and_then(|value| value.lock().ok())
                        && let Some(sender) = sender.as_ref()
                    {
                        let _ = sender.try_send(HookEvent::Cursor(cursor));
                    }
                } else if let Some((button, phase, click_count)) = classify(message) {
                    let event = MouseEvent {
                        timestamp_micros,
                        screen_x: data.pt.x,
                        screen_y: data.pt.y,
                        button: Some(button),
                        phase: Some(phase),
                        wheel_delta: if message == WM_MOUSEWHEEL {
                            (data.mouseData >> 16) as i16
                        } else {
                            0
                        },
                        click_count,
                    };
                    if let Some(sender) = SENDER.get().and_then(|value| value.lock().ok())
                        && let Some(sender) = sender.as_ref()
                    {
                        let _ = sender.try_send(HookEvent::Mouse(event));
                    }
                }
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    fn classify(message: u32) -> Option<(MouseButton, ButtonPhase, u8)> {
        match message {
            WM_LBUTTONDOWN => Some((MouseButton::Left, ButtonPhase::Press, 1)),
            WM_LBUTTONUP => Some((MouseButton::Left, ButtonPhase::Release, 1)),
            WM_LBUTTONDBLCLK => Some((MouseButton::Left, ButtonPhase::Press, 2)),
            WM_RBUTTONDOWN => Some((MouseButton::Right, ButtonPhase::Press, 1)),
            WM_RBUTTONUP => Some((MouseButton::Right, ButtonPhase::Release, 1)),
            WM_RBUTTONDBLCLK => Some((MouseButton::Right, ButtonPhase::Press, 2)),
            WM_MBUTTONDOWN => Some((MouseButton::Middle, ButtonPhase::Press, 1)),
            WM_MBUTTONUP => Some((MouseButton::Middle, ButtonPhase::Release, 1)),
            WM_MBUTTONDBLCLK => Some((MouseButton::Middle, ButtonPhase::Press, 2)),
            WM_XBUTTONDOWN => Some((MouseButton::X1, ButtonPhase::Press, 1)),
            WM_XBUTTONUP => Some((MouseButton::X1, ButtonPhase::Release, 1)),
            WM_XBUTTONDBLCLK => Some((MouseButton::X1, ButtonPhase::Press, 2)),
            WM_MOUSEWHEEL => Some((MouseButton::Middle, ButtonPhase::Press, 0)),
            _ => None,
        }
    }

    pub struct InputRecordingHandle {
        hook_thread: Option<JoinHandle<Result<(), InputError>>>,
        writer_thread: Option<JoinHandle<Result<u64, InputError>>>,
        thread_id: u32,
    }

    impl InputRecordingHandle {
        pub fn stop(mut self) -> Result<u64, InputError> {
            unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) }
                .map_err(|error| InputError::Native(error.to_string()))?;
            self.hook_thread
                .take()
                .expect("hook thread exists")
                .join()
                .map_err(|_| InputError::Native("mouse hook thread panicked".into()))??;
            if let Ok(mut sender) = SENDER.get().expect("sender initialized").lock() {
                sender.take();
            }
            self.writer_thread
                .take()
                .expect("writer thread exists")
                .join()
                .map_err(|_| InputError::Native("telemetry writer thread panicked".into()))?
        }
    }

    pub fn start(
        cursor_path: PathBuf,
        clicks_path: PathBuf,
        source: RectI32,
        offset_micros: i64,
    ) -> Result<InputRecordingHandle, InputError> {
        let (sender, receiver) = sync_channel(8192);
        *SENDER
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(|_| InputError::Native("sender mutex poisoned".into()))? = Some(sender);
        *CONTEXT
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(|_| InputError::Native("context mutex poisoned".into()))? =
            Some(HookContext {
                source,
                offset_micros,
                started: Instant::now(),
            });
        let writer_thread = std::thread::Builder::new()
            .name("kiri-input-writer".into())
            .spawn(move || writer_loop(receiver, cursor_path, clicks_path))
            .map_err(|error| InputError::Native(error.to_string()))?;
        let (thread_sender, thread_receiver) = std::sync::mpsc::channel();
        let hook_thread = std::thread::Builder::new()
            .name("kiri-input-hook".into())
            .spawn(move || {
                let thread_id = unsafe { GetCurrentThreadId() };
                let hook: HHOOK =
                    unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), None, 0) }
                        .map_err(|error| InputError::Native(error.to_string()))?;
                thread_sender
                    .send(thread_id)
                    .map_err(|_| InputError::Native("failed to publish hook thread".into()))?;
                let mut message = MSG::default();
                while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {}
                unsafe { UnhookWindowsHookEx(hook) }
                    .map_err(|error| InputError::Native(error.to_string()))?;
                Ok(())
            })
            .map_err(|error| InputError::Native(error.to_string()))?;
        let thread_id = thread_receiver
            .recv()
            .map_err(|_| InputError::Native("mouse hook failed to start".into()))?;
        Ok(InputRecordingHandle {
            hook_thread: Some(hook_thread),
            writer_thread: Some(writer_thread),
            thread_id,
        })
    }

    fn writer_loop(
        receiver: Receiver<HookEvent>,
        cursor_path: PathBuf,
        clicks_path: PathBuf,
    ) -> Result<u64, InputError> {
        if let Some(parent) = cursor_path.parent() {
            fs::create_dir_all(parent).map_err(|e| InputError::Native(e.to_string()))?;
        }
        let mut cursor = BufWriter::new(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(cursor_path)
                .map_err(|e| InputError::Native(e.to_string()))?,
        );
        let mut clicks = BufWriter::new(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(clicks_path)
                .map_err(|e| InputError::Native(e.to_string()))?,
        );
        let mut count = 0;
        while let Ok(event) = receiver.recv() {
            let writer = match event {
                HookEvent::Cursor(value) => {
                    serde_json::to_writer(&mut cursor, &value)
                        .map_err(|e| InputError::Native(e.to_string()))?;
                    &mut cursor
                }
                HookEvent::Mouse(value) => {
                    serde_json::to_writer(&mut clicks, &value)
                        .map_err(|e| InputError::Native(e.to_string()))?;
                    &mut clicks
                }
            };
            writer
                .write_all(
                    b"
",
                )
                .map_err(|e| InputError::Native(e.to_string()))?;
            count += 1;
        }
        cursor
            .flush()
            .map_err(|e| InputError::Native(e.to_string()))?;
        clicks
            .flush()
            .map_err(|e| InputError::Native(e.to_string()))?;
        Ok(count)
    }
}

#[cfg(windows)]
pub use recorder::{InputRecordingHandle, start as start_recording};
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_relative_mapping_preserves_screen_coordinates() {
        let sample = CursorSample::map(
            42,
            960,
            540,
            RectI32 {
                left: 0,
                top: 0,
                width: 1920,
                height: 1080,
            },
        );
        assert_eq!((sample.screen_x, sample.screen_y), (960, 540));
        assert_eq!((sample.normalized.x, sample.normalized.y), (0.5, 0.5));
    }
}
