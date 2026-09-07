use crate::{CaptureError, CaptureSource, CaptureSourceKind, RectI32, SourceAvailability};
use windows::Win32::{
    Foundation::RECT,
    Graphics::Gdi::{GetMonitorInfoW, MONITORINFO},
    UI::{
        HiDpi::{GetDpiForSystem, GetDpiForWindow},
        WindowsAndMessaging::{IsIconic, IsWindow},
    },
};
use windows_capture::{monitor::Monitor, window::Window};

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
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        unsafe { GetMonitorInfoW(raw, &mut info) }.map_err(|e| CaptureError::Native(e.to_string()))?;
        result.push(CaptureSource {
            id: format!("display:{}", monitor.device_name().unwrap_or_else(|_| format!("{}", raw.0 as usize))),
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
        let hwnd = window.as_raw_hwnd();
        let availability = if !unsafe { IsWindow(hwnd).as_bool() } {
            SourceAvailability::Closed
        } else if unsafe { IsIconic(hwnd).as_bool() } {
            SourceAvailability::Minimized
        } else if window.is_valid() {
            SourceAvailability::Available
        } else {
            SourceAvailability::Invalid
        };
        let bounds = window.rect().map(rect).unwrap_or(RectI32 { left: 0, top: 0, width: 0, height: 0 });
        result.push(CaptureSource {
            id: format!("window:{}", hwnd.0 as usize),
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