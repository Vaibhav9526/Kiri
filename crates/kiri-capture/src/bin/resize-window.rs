#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use windows::Win32::{
        Foundation::{HWND, RECT},
        UI::WindowsAndMessaging::{GetWindowRect, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos},
    };
    let raw_text = std::env::args().nth(1).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "window handle argument required (HWND as integer)",
        )
    })?;
    let raw: isize = raw_text.parse().map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid window handle '{raw_text}'; expected an integer HWND"),
        )
    })?;
    let hwnd = HWND(raw as *mut _);
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect)? };
    // `right - left` in `i32` can overflow for hostile rects; saturate so a
    // corrupt rect restores to a sane size instead of panicking in debug.
    let restore_width = (i64::from(rect.right) - i64::from(rect.left)).clamp(1, 16_384) as i32;
    let restore_height = (i64::from(rect.bottom) - i64::from(rect.top)).clamp(1, 16_384) as i32;
    std::thread::sleep(std::time::Duration::from_secs(2));
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            rect.left,
            rect.top,
            1000,
            650,
            SWP_NOACTIVATE | SWP_NOZORDER,
        )?
    };
    std::thread::sleep(std::time::Duration::from_secs(3));
    unsafe {
        SetWindowPos(
            hwnd,
            None,
            rect.left,
            rect.top,
            restore_width,
            restore_height,
            SWP_NOACTIVATE | SWP_NOZORDER,
        )?
    };
    Ok(())
}
#[cfg(not(windows))]
fn main() {}

#[cfg(test)]
mod tests {
    #[test]
    fn restore_extent_math_saturates_without_panicking() {
        // Mirrors the `main` restore-size math: hostile rects clamp instead
        // of overflowing `i32` subtraction in debug builds.
        let width = (i64::from(i32::MAX) - i64::from(i32::MIN)).clamp(1, 16_384);
        assert_eq!(width, 16_384);
        let width = (i64::from(100) - i64::from(0)).clamp(1, 16_384);
        assert_eq!(width, 100);
    }
}
