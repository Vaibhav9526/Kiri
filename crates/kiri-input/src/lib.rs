use kiri_capture::{NormalizedPoint, RectI32};
use serde::{Deserialize, Serialize};
use std::path::Path;
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

/// Cursor shape at sample time, aligned with Recordly's `cursorType`
/// vocabulary (`arrow`, `text`, `pointer`, `crosshair`, `open-hand`,
/// `closed-hand`, `resize-ew`, `resize-ns`, `not-allowed`) so downstream
/// renderers can share cursor assets. Serialized as `cursorType` via the
/// struct-level `camelCase` rename; `None` means hidden/unknown and is
/// skipped on the wire.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CursorType {
    #[default]
    Arrow,
    Text,
    Pointer,
    Crosshair,
    OpenHand,
    ClosedHand,
    ResizeEw,
    ResizeNs,
    NotAllowed,
}

impl CursorType {
    #[must_use]
    pub fn as_recordly_str(self) -> &'static str {
        match self {
            Self::Arrow => "arrow",
            Self::Text => "text",
            Self::Pointer => "pointer",
            Self::Crosshair => "crosshair",
            Self::OpenHand => "open-hand",
            Self::ClosedHand => "closed-hand",
            Self::ResizeEw => "resize-ew",
            Self::ResizeNs => "resize-ns",
            Self::NotAllowed => "not-allowed",
        }
    }

    #[must_use]
    pub fn from_recordly_str(value: &str) -> Option<Self> {
        match value {
            "arrow" => Some(Self::Arrow),
            "text" | "ibeam" => Some(Self::Text),
            "pointer" | "hand" => Some(Self::Pointer),
            "crosshair" => Some(Self::Crosshair),
            "open-hand" | "sizeall" => Some(Self::OpenHand),
            "closed-hand" => Some(Self::ClosedHand),
            "resize-ew" | "sizewe" => Some(Self::ResizeEw),
            "resize-ns" | "sizens" => Some(Self::ResizeNs),
            "not-allowed" | "no" => Some(Self::NotAllowed),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorSample {
    #[serde(alias = "timestamp_micros", alias = "timeMicros")]
    pub timestamp_micros: i64,
    #[serde(alias = "screen_x", alias = "screenX")]
    pub screen_x: i32,
    #[serde(alias = "screen_y", alias = "screenY")]
    pub screen_y: i32,
    pub normalized: NormalizedPoint,
    /// Recordly-aligned `cursorType`. `None` on old files / hidden cursor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_type: Option<CursorType>,
}

impl CursorSample {
    #[must_use]
    pub fn map(timestamp_micros: i64, screen_x: i32, screen_y: i32, source: RectI32) -> Self {
        Self {
            timestamp_micros,
            screen_x,
            screen_y,
            normalized: source.normalize(screen_x, screen_y),
            cursor_type: None,
        }
    }

    #[must_use]
    pub fn with_cursor_type(mut self, cursor_type: Option<CursorType>) -> Self {
        self.cursor_type = cursor_type;
        self
    }

    /// Recordly `timeMs` view (millis, floored toward negative infinity).
    #[must_use]
    pub fn time_ms(&self) -> i64 {
        self.timestamp_micros.div_euclid(1_000)
    }

    /// Recordly `cx` view (normalized x).
    #[must_use]
    pub fn cx(&self) -> f64 {
        self.normalized.x
    }

    /// Recordly `cy` view (normalized y).
    #[must_use]
    pub fn cy(&self) -> f64 {
        self.normalized.y
    }

    /// Recordly-style point `{timeMs,cx,cy,cursorType}` for shared renderers.
    #[must_use]
    pub fn to_recordly_value(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        map.insert("timeMs".into(), serde_json::Value::from(self.time_ms()));
        map.insert("cx".into(), serde_json::json!(self.normalized.x));
        map.insert("cy".into(), serde_json::json!(self.normalized.y));
        if let Some(cursor_type) = self.cursor_type {
            map.insert(
                "cursorType".into(),
                serde_json::Value::from(cursor_type.as_recordly_str()),
            );
        }
        serde_json::Value::Object(map)
    }

    /// Build a sample from a Recordly `{timeMs,cx,cy,cursorType}` point.
    /// `cx`/`cy` are clamped like Recordly's normalizer (non-finite → 0.5);
    /// `screen_x`/`screen_y` are reconstructed from `source` so clicks stay
    /// aligned with Kiri's pixel coordinates.
    #[must_use]
    pub fn from_recordly_point(
        time_ms: f64,
        cx: f64,
        cy: f64,
        cursor_type: Option<CursorType>,
        source: RectI32,
    ) -> Self {
        let timestamp_micros = if time_ms.is_finite() {
            (time_ms.max(0.0) * 1000.0).round().min(i64::MAX as f64) as i64
        } else {
            0
        };
        let nx = clamp01_or_half(cx);
        let ny = clamp01_or_half(cy);
        let screen_x = source
            .left
            .saturating_add((f64::from(source.width) * nx).round() as i32);
        let screen_y = source
            .top
            .saturating_add((f64::from(source.height) * ny).round() as i32);
        Self {
            timestamp_micros,
            screen_x,
            screen_y,
            normalized: source.normalize(screen_x, screen_y),
            cursor_type,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MouseEvent {
    #[serde(alias = "timestamp_micros", alias = "timeMicros")]
    pub timestamp_micros: i64,
    #[serde(alias = "screen_x", alias = "screenX")]
    pub screen_x: i32,
    #[serde(alias = "screen_y", alias = "screenY")]
    pub screen_y: i32,
    pub button: Option<MouseButton>,
    pub phase: Option<ButtonPhase>,
    #[serde(alias = "wheel_delta", alias = "wheelDelta")]
    pub wheel_delta: i16,
    #[serde(alias = "click_count", alias = "clickCount")]
    pub click_count: u8,
    /// Recordly-aligned `cursorType` at click time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor_type: Option<CursorType>,
}

impl MouseEvent {
    #[must_use]
    pub fn time_ms(&self) -> i64 {
        self.timestamp_micros.div_euclid(1_000)
    }

    #[must_use]
    pub fn is_wheel(&self) -> bool {
        self.wheel_delta != 0 && self.button.is_none()
    }
}

#[derive(Debug, Error)]
pub enum InputError {
    #[error("input telemetry is only available on Windows")]
    Unsupported,
    #[error("input telemetry failed: {0}")]
    Native(String),
}

// ---------------------------------------------------------------------------
// Pure, cross-platform helpers (unit-testable without a hook)
// ---------------------------------------------------------------------------

/// Recordly `normalizeCursorTelemetrySamples` policy: finite values clamp to
/// 0..=1, non-finite fall back to 0.5 (center) so one NaN never breaks zoom.
#[must_use]
pub fn clamp01_or_half(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.5
    }
}

/// Recordly `timeMs` policy: finite values floor at 0, non-finite become 0.
#[must_use]
pub fn normalize_time_ms(value: f64) -> i64 {
    if value.is_finite() {
        value.max(0.0).round().min(i64::MAX as f64) as i64
    } else {
        0
    }
}

// Raw `WM_*` values so classification stays testable without the `windows`
// crate (values match `WindowsAndMessaging`).
pub const WM_MOUSEMOVE: u32 = 0x0200;
pub const WM_LBUTTONDOWN: u32 = 0x0201;
pub const WM_LBUTTONUP: u32 = 0x0202;
pub const WM_LBUTTONDBLCLK: u32 = 0x0203;
pub const WM_RBUTTONDOWN: u32 = 0x0204;
pub const WM_RBUTTONUP: u32 = 0x0205;
pub const WM_RBUTTONDBLCLK: u32 = 0x0206;
pub const WM_MBUTTONDOWN: u32 = 0x0207;
pub const WM_MBUTTONUP: u32 = 0x0208;
pub const WM_MBUTTONDBLCLK: u32 = 0x0209;
pub const WM_MOUSEWHEEL: u32 = 0x020A;
pub const WM_XBUTTONDOWN: u32 = 0x020B;
pub const WM_XBUTTONUP: u32 = 0x020C;
pub const WM_XBUTTONDBLCLK: u32 = 0x020D;
pub const WM_MOUSEHWHEEL: u32 = 0x020E;

/// True for vertical + horizontal wheel messages (reported as wheel deltas,
/// never as synthetic clicks).
#[must_use]
pub fn is_wheel_message(message: u32) -> bool {
    message == WM_MOUSEWHEEL || message == WM_MOUSEHWHEEL
}

/// Classify a button message. `hi_word` is `HIWORD(mouseData)`: wheel delta
/// for wheel messages (unused here), `1`/`2` for XBUTTON1/XBUTTON2.
/// Returns `None` for moves, wheel, and unknown messages so callers never
/// synthesize phantom middle-clicks for wheel events.
#[must_use]
pub fn classify_button(message: u32, hi_word: u16) -> Option<(MouseButton, ButtonPhase, u8)> {
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
        WM_XBUTTONDOWN => Some((
            if hi_word == 2 {
                MouseButton::X2
            } else {
                MouseButton::X1
            },
            ButtonPhase::Press,
            1,
        )),
        WM_XBUTTONUP => Some((
            if hi_word == 2 {
                MouseButton::X2
            } else {
                MouseButton::X1
            },
            ButtonPhase::Release,
            1,
        )),
        WM_XBUTTONDBLCLK => Some((
            if hi_word == 2 {
                MouseButton::X2
            } else {
                MouseButton::X1
            },
            ButtonPhase::Press,
            2,
        )),
        _ => None,
    }
}

/// Extract `HIWORD(mouseData)` (X-button id / wheel delta high word).
#[must_use]
pub fn hi_word(mouse_data: u32) -> u16 {
    (mouse_data >> 16) as u16
}

/// Extract the signed wheel delta (`GET_WHEEL_DELTA_WPARAM`).
#[must_use]
pub fn wheel_delta_from_mouse_data(mouse_data: u32) -> i16 {
    hi_word(mouse_data) as i16
}

/// Decide whether a telemetry start should truncate stale files or append to
/// the current session. `offset_micros <= 0` is always fresh (first segment).
/// Otherwise, if either file already holds samples newer than `offset_micros`
/// the files belong to a previous session reusing the same path, so truncate
/// instead of mixing two sessions. Best-effort: unreadable files → append
/// (which creates them), never fail the start path.
#[must_use]
pub fn should_truncate_telemetry(
    offset_micros: i64,
    cursor_path: &Path,
    clicks_path: &Path,
) -> bool {
    if offset_micros <= 0 {
        return true;
    }
    for path in [cursor_path, clicks_path] {
        if let Some(last) = last_timestamp_micros_in_file(path)
            && last > offset_micros
        {
            return true;
        }
    }
    false
}

/// Best-effort last `timestampMicros` (or Recordly `timeMs`) in a JSONL file.
/// Returns `None` for missing/empty/unreadable files. Skips blank + corrupt
/// lines like the lenient readers below.
#[must_use]
pub fn last_timestamp_micros_in_file(path: &Path) -> Option<i64> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut last: Option<i64> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(micros) = value
            .get("timestampMicros")
            .or_else(|| value.get("timestamp_micros"))
            .and_then(serde_json::Value::as_i64)
        {
            last = Some(micros);
        } else if let Some(ms) = value.get("timeMs").and_then(|v| v.as_f64()) {
            last = Some(normalize_time_ms(ms) * 1000);
        }
    }
    last
}

/// Lenient cursor reader: accepts current Kiri rows
/// (`timestampMicros/screenX/screenY/normalized`), snake_case aliases, and
/// Recordly `{timeMs,cx,cy,cursorType}` rows. Skips blank/corrupt lines,
/// clamps `cx`/`cy`, sorts by timestamp. `source` reconstructs pixel
/// coordinates for Recordly-shaped rows.
pub fn read_cursor_samples_lenient(
    path: &Path,
    source: RectI32,
) -> Result<Vec<CursorSample>, InputError> {
    let text = std::fs::read_to_string(path).map_err(|e| InputError::Native(e.to_string()))?;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("timestampMicros").is_some() || value.get("timestamp_micros").is_some() {
            if let Ok(sample) = serde_json::from_value::<CursorSample>(value) {
                out.push(sample);
            }
        } else if value.get("timeMs").is_some() {
            let time_ms = value
                .get("timeMs")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            let cx = value
                .get("cx")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.5);
            let cy = value
                .get("cy")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.5);
            let cursor_type = value
                .get("cursorType")
                .and_then(serde_json::Value::as_str)
                .and_then(CursorType::from_recordly_str);
            out.push(CursorSample::from_recordly_point(
                time_ms,
                cx,
                cy,
                cursor_type,
                source,
            ));
        }
    }
    out.sort_by_key(|sample| sample.timestamp_micros);
    Ok(out)
}

/// Lenient clicks reader: accepts Kiri `MouseEvent` rows (camelCase +
/// snake_case aliases) and skips blank/corrupt lines. Recordly embeds clicks
/// as cursor `interactionType`s; those rows live in `cursor.jsonl` and are
/// intentionally not duplicated here.
pub fn read_mouse_events_lenient(path: &Path) -> Result<Vec<MouseEvent>, InputError> {
    let text = std::fs::read_to_string(path).map_err(|e| InputError::Native(e.to_string()))?;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("timestampMicros").is_some()
            || value.get("timestamp_micros").is_some()
            || value.get("timeMs").is_some()
        {
            // Accept Kiri rows directly; accept Recordly `timeMs` rows that
            // also carry button/wheel fields (forward-compat), else skip.
            if value.get("timestampMicros").is_some() || value.get("timestamp_micros").is_some() {
                if let Ok(event) = serde_json::from_value::<MouseEvent>(value) {
                    out.push(event);
                }
            } else if let Ok(event) = serde_json::from_value::<MouseEvent>(value) {
                out.push(event);
            }
        }
    }
    out.sort_by_key(|event| event.timestamp_micros);
    Ok(out)
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
            atomic::{AtomicBool, Ordering},
            mpsc::{Receiver, SyncSender, sync_channel},
        },
        thread::JoinHandle,
        time::{Duration, Instant},
    };
    use windows::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::Threading::GetCurrentThreadId,
        UI::WindowsAndMessaging::{
            CURSOR_SHOWING, CURSORINFO, CallNextHookEx, GetCursorInfo, GetMessageW, HCURSOR, HHOOK,
            IDC_APPSTARTING, IDC_ARROW, IDC_CROSS, IDC_HAND, IDC_IBEAM, IDC_NO, IDC_SIZEALL,
            IDC_SIZENS, IDC_SIZEWE, IDC_WAIT, LoadCursorW, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE,
            PeekMessageW, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx, WH_MOUSE_LL,
            WM_QUIT,
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
    /// Guards the process-wide low-level hook: only one input session may own
    /// the hook chain at a time. Prevents double-start cross-contamination
    /// where a second session overwrites `SENDER`/`CONTEXT` while the first
    /// hook thread is still chained.
    static ACTIVE: AtomicBool = AtomicBool::new(false);

    const HOOK_STOP_TIMEOUT: Duration = Duration::from_secs(5);
    const WRITER_STOP_TIMEOUT: Duration = Duration::from_secs(5);

    /// Best-effort cursor shape via `GetCursorInfo` + handle comparison
    /// (Recordly's `cursor-monitor` polls the same API every 50ms). Never
    /// fails the hook: hidden cursor / errors yield `None`.
    fn current_cursor_type() -> Option<CursorType> {
        static HANDLES: OnceLock<Vec<(HCURSOR, CursorType)>> = OnceLock::new();
        let handles = HANDLES.get_or_init(|| {
            let mut map = Vec::new();
            let pairs = [
                (IDC_ARROW, CursorType::Arrow),
                (IDC_IBEAM, CursorType::Text),
                (IDC_HAND, CursorType::Pointer),
                (IDC_CROSS, CursorType::Crosshair),
                (IDC_SIZEWE, CursorType::ResizeEw),
                (IDC_SIZENS, CursorType::ResizeNs),
                (IDC_SIZEALL, CursorType::OpenHand),
                (IDC_NO, CursorType::NotAllowed),
                (IDC_WAIT, CursorType::Arrow),
                (IDC_APPSTARTING, CursorType::Arrow),
            ];
            for (id, cursor_type) in pairs {
                if let Ok(handle) = unsafe { LoadCursorW(None, id) } {
                    map.push((handle, cursor_type));
                }
            }
            map
        });
        let mut info = CURSORINFO {
            cbSize: std::mem::size_of::<CURSORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetCursorInfo(&mut info) }.is_err() {
            return None;
        }
        if info.flags & CURSOR_SHOWING.0 == 0 {
            return None;
        }
        for (handle, cursor_type) in handles {
            if *handle == info.hCursor {
                return Some(*cursor_type);
            }
        }
        Some(CursorType::Arrow)
    }

    fn emit(event: HookEvent) {
        if let Some(lock) = SENDER.get()
            && let Ok(sender) = lock.lock()
            && let Some(sender) = sender.as_ref()
        {
            let _ = sender.try_send(event);
        }
    }

    unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let data = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
            let context = CONTEXT
                .get()
                .and_then(|value| value.lock().ok())
                .and_then(|value| *value);
            if let Some(context) = context {
                let timestamp_micros = context.offset_micros.saturating_add(
                    context.started.elapsed().as_micros().min(i64::MAX as u128) as i64,
                );
                let message = wparam.0 as u32;
                // `WM_MOUSEMOVE` in the local constants matches the Win32 value;
                // compare as `u32` so the pure `classify_button` table stays the
                // single source of truth for button messages.
                if message == super::WM_MOUSEMOVE {
                    let cursor =
                        CursorSample::map(timestamp_micros, data.pt.x, data.pt.y, context.source)
                            .with_cursor_type(current_cursor_type());
                    emit(HookEvent::Cursor(cursor));
                } else if super::is_wheel_message(message) {
                    // Wheel reports a delta, never a synthetic middle-click
                    // (the old code emitted `Middle/Press` with count 0,
                    // polluting `clicks.jsonl` with phantom clicks).
                    emit(HookEvent::Mouse(MouseEvent {
                        timestamp_micros,
                        screen_x: data.pt.x,
                        screen_y: data.pt.y,
                        button: None,
                        phase: None,
                        wheel_delta: super::wheel_delta_from_mouse_data(data.mouseData),
                        click_count: 0,
                        cursor_type: current_cursor_type(),
                    }));
                } else if let Some((button, phase, click_count)) =
                    super::classify_button(message, super::hi_word(data.mouseData))
                {
                    emit(HookEvent::Mouse(MouseEvent {
                        timestamp_micros,
                        screen_x: data.pt.x,
                        screen_y: data.pt.y,
                        button: Some(button),
                        phase: Some(phase),
                        wheel_delta: 0,
                        click_count,
                        cursor_type: current_cursor_type(),
                    }));
                }
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    pub struct InputRecordingHandle {
        hook_thread: Option<JoinHandle<Result<(), InputError>>>,
        writer_thread: Option<JoinHandle<Result<u64, InputError>>>,
        thread_id: u32,
    }

    fn post_quit(thread_id: u32) {
        // The hook thread creates its message queue before publishing
        // `thread_id`, so one post normally suffices; retry briefly in case
        // the queue is momentarily busy.
        for _ in 0..20 {
            if unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) }.is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn join_with_timeout<T>(thread: JoinHandle<T>, timeout: Duration) -> Result<T, InputError> {
        let deadline = Instant::now() + timeout;
        // `JoinHandle::is_finished` is the only non-blocking poll; spin
        // briefly rather than hanging pause/stop on a wedged device thread.
        // A panic becomes `InputError::Native`, never a propagated panic.
        while Instant::now() < deadline {
            if thread.is_finished() {
                return match thread.join() {
                    Ok(value) => Ok(value),
                    Err(_) => Err(InputError::Native("input thread panicked".into())),
                };
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // Timeout: caller decides (forget to avoid hanging forever).
        // This branch is unreachable without consuming `thread`, so callers
        // use `is_finished` polling inline; kept for documentation.
        match thread.join() {
            Ok(value) => Ok(value),
            Err(_) => Err(InputError::Native("input thread panicked".into())),
        }
    }

    impl InputRecordingHandle {
        pub fn stop(mut self) -> Result<u64, InputError> {
            post_quit(self.thread_id);
            if let Some(thread) = self.hook_thread.take() {
                match join_with_timeout(thread, HOOK_STOP_TIMEOUT) {
                    Ok(result) => result?,
                    Err(thread) => {
                        // Never hang stop forever on a wedged hook; detach so
                        // the coordinator can still finalize other tracks and
                        // report this one as a warning. `ACTIVE` stays set
                        // until the thread actually exits, so a new session
                        // fails fast instead of chaining a second hook.
                        std::mem::forget(thread);
                        if let Some(lock) = SENDER.get()
                            && let Ok(mut sender) = lock.lock()
                        {
                            sender.take();
                        }
                        return Err(InputError::Native(
                            "mouse hook thread did not exit within 5s; device may be wedged".into(),
                        ));
                    }
                }
            }
            if let Some(lock) = SENDER.get()
                && let Ok(mut sender) = lock.lock()
            {
                sender.take();
            }
            if let Some(thread) = self.writer_thread.take() {
                match join_with_timeout(thread, WRITER_STOP_TIMEOUT) {
                    Ok(result) => {
                        ACTIVE.store(false, Ordering::Release);
                        return result;
                    }
                    Err(thread) => {
                        std::mem::forget(thread);
                        ACTIVE.store(false, Ordering::Release);
                        return Err(InputError::Native(
                            "telemetry writer thread did not exit within 5s".into(),
                        ));
                    }
                }
            }
            ACTIVE.store(false, Ordering::Release);
            Ok(0)
        }
    }

    impl Drop for InputRecordingHandle {
        fn drop(&mut self) {
            // Best-effort unblock of the hook message loop so a dropped handle
            // never leaves a global hook thread behind. `ACTIVE` is cleared
            // by the exiting hook thread / `stop`, never here, so a new
            // `start` during shutdown fails fast instead of double-chaining.
            post_quit(self.thread_id);
            if let Some(lock) = SENDER.get()
                && let Ok(mut sender) = lock.lock()
            {
                sender.take();
            }
            // Reap already-finished threads without blocking Drop.
            if let Some(thread) = self.hook_thread.take()
                && thread.is_finished()
            {
                let _ = thread.join();
            }
            if let Some(thread) = self.writer_thread.take() {
                if thread.is_finished() {
                    let _ = thread.join();
                    ACTIVE.store(false, Ordering::Release);
                } else {
                    std::mem::forget(thread);
                }
            }
        }
    }

    /// Update the source rectangle mid-recording (window move / DPI change,
    /// REC-010). Takes effect for subsequently hooked events; already-written
    /// samples keep their original normalization.
    pub fn update_source(source: RectI32) -> Result<(), InputError> {
        if source.width <= 0 || source.height <= 0 {
            return Err(InputError::Native(
                "capture source has no visible area; restore the window before recording".into(),
            ));
        }
        let Some(lock) = CONTEXT.get() else {
            return Err(InputError::Native("no input recording is active".into()));
        };
        let mut guard = lock
            .lock()
            .map_err(|_| InputError::Native("context mutex poisoned".into()))?;
        match guard.as_mut() {
            Some(context) => {
                context.source = source;
                Ok(())
            }
            None => Err(InputError::Native("no input recording is active".into())),
        }
    }

    /// Whether an input session currently owns the global hook.
    #[must_use]
    pub fn is_recording() -> bool {
        ACTIVE.load(Ordering::Acquire)
    }

    pub fn start(
        cursor_path: PathBuf,
        clicks_path: PathBuf,
        source: RectI32,
        offset_micros: i64,
    ) -> Result<InputRecordingHandle, InputError> {
        if source.width <= 0 || source.height <= 0 {
            return Err(InputError::Native(
                "capture source has no visible area; restore the window before recording".into(),
            ));
        }
        // Refuse a second concurrent session instead of overwriting the
        // globals and cross-contaminating both writers (rapid start/stop +
        // pause/resume safety).
        if ACTIVE.swap(true, Ordering::AcqRel) {
            return Err(InputError::Native(
                "input telemetry is already recording; stop it before starting a new session"
                    .into(),
            ));
        }
        let fresh_session =
            super::should_truncate_telemetry(offset_micros, &cursor_path, &clicks_path);
        let (sender, receiver) = sync_channel(8192);
        *SENDER
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(|_| {
                ACTIVE.store(false, Ordering::Release);
                InputError::Native("sender mutex poisoned".into())
            })? = Some(sender);
        *CONTEXT
            .get_or_init(|| Mutex::new(None))
            .lock()
            .map_err(|_| {
                ACTIVE.store(false, Ordering::Release);
                InputError::Native("context mutex poisoned".into())
            })? = Some(HookContext {
            source,
            offset_micros,
            started: Instant::now(),
        });
        let writer_cursor = cursor_path.clone();
        let writer_clicks = clicks_path.clone();
        let writer_thread = match std::thread::Builder::new()
            .name("kiri-input-writer".into())
            .spawn(move || writer_loop(receiver, writer_cursor, writer_clicks, fresh_session))
        {
            Ok(thread) => thread,
            Err(error) => {
                cleanup_after_failed_start();
                return Err(InputError::Native(error.to_string()));
            }
        };
        let (thread_sender, thread_receiver) = std::sync::mpsc::channel();
        let hook_thread = match std::thread::Builder::new()
            .name("kiri-input-hook".into())
            .spawn(move || {
                let thread_id = unsafe { GetCurrentThreadId() };
                let hook: HHOOK =
                    unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), None, 0) }
                        .map_err(|error| InputError::Native(error.to_string()))?;
                // Create this thread's message queue BEFORE publishing the
                // thread id: `PostThreadMessageW` fails when the target has
                // no queue yet, which previously lost the QUIT and leaked the
                // hook thread forever on fast stop().
                let mut queue = MSG::default();
                unsafe {
                    PeekMessageW(&mut queue, Some(HWND::default()), 0, 0, PM_NOREMOVE);
                }
                thread_sender
                    .send(thread_id)
                    .map_err(|_| InputError::Native("failed to publish hook thread".into()))?;
                struct Guard {
                    hook: HHOOK,
                }
                impl Drop for Guard {
                    fn drop(&mut self) {
                        unsafe {
                            let _ = UnhookWindowsHookEx(self.hook);
                        }
                        ACTIVE.store(false, Ordering::Release);
                    }
                }
                let _guard = Guard { hook };
                let mut message = MSG::default();
                while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {}
                Ok(())
            }) {
            Ok(thread) => thread,
            Err(error) => {
                // Unhook never installed; drop the sender so the writer exits,
                // join it so no thread leaks, then release the session.
                cleanup_after_failed_start();
                let _ = writer_thread.join();
                ACTIVE.store(false, Ordering::Release);
                return Err(InputError::Native(error.to_string()));
            }
        };
        let thread_id = match thread_receiver.recv_timeout(Duration::from_secs(10)) {
            Ok(id) => id,
            Err(_) => {
                // Hook thread never published (install wedged/failed):
                // unblock it, reap the writer, release globals.
                cleanup_after_failed_start();
                post_quit_hint();
                let _ = writer_thread.join();
                let _ = hook_thread.join();
                ACTIVE.store(false, Ordering::Release);
                return Err(InputError::Native("mouse hook failed to start".into()));
            }
        };
        Ok(InputRecordingHandle {
            hook_thread: Some(hook_thread),
            writer_thread: Some(writer_thread),
            thread_id,
        })
    }

    /// Clear globals after a failed `start` so the next attempt sees a clean
    /// slate (no stale sender/context steering events into a dead writer).
    fn cleanup_after_failed_start() {
        if let Some(lock) = SENDER.get()
            && let Ok(mut sender) = lock.lock()
        {
            sender.take();
        }
        if let Some(lock) = CONTEXT.get()
            && let Ok(mut context) = lock.lock()
        {
            context.take();
        }
    }

    /// Best-effort unblock when the hook thread id was never published.
    /// Broadcast is impossible without the id, so this only drops the sender
    /// (the writer then exits); the hook thread's own guard still unhooks on
    /// process exit.
    fn post_quit_hint() {
        cleanup_after_failed_start();
    }

    fn writer_loop(
        receiver: Receiver<HookEvent>,
        cursor_path: PathBuf,
        clicks_path: PathBuf,
        fresh_session: bool,
    ) -> Result<u64, InputError> {
        if let Some(parent) = cursor_path.parent() {
            fs::create_dir_all(parent).map_err(|e| InputError::Native(e.to_string()))?;
        }
        if let Some(parent) = clicks_path.parent() {
            fs::create_dir_all(parent).map_err(|e| InputError::Native(e.to_string()))?;
        }
        // First segment of a session truncates stale telemetry so a reused
        // project path never mixes two sessions; later segments (pause/resume)
        // append to the same `cursor.jsonl` / `clicks.jsonl`.
        let open = |path: &PathBuf| {
            if fresh_session {
                OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(path)
            } else {
                OpenOptions::new().create(true).append(true).open(path)
            }
            .map_err(|e| InputError::Native(e.to_string()))
        };
        let mut cursor = BufWriter::new(open(&cursor_path)?);
        let mut clicks = BufWriter::new(open(&clicks_path)?);
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
            // Flush per event so a crash still leaves recoverable telemetry.
            writer
                .flush()
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

#[cfg(not(windows))]
mod unsupported {
    use super::{InputError, RectI32};
    use std::path::PathBuf;

    pub struct InputRecordingHandle;

    impl InputRecordingHandle {
        pub fn stop(self) -> Result<u64, InputError> {
            Err(InputError::Unsupported)
        }
    }

    pub fn start_recording(
        _: PathBuf,
        _: PathBuf,
        _: RectI32,
        _: i64,
    ) -> Result<InputRecordingHandle, InputError> {
        Err(InputError::Unsupported)
    }
}

#[cfg(not(windows))]
pub use unsupported::{InputRecordingHandle, start_recording};
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
