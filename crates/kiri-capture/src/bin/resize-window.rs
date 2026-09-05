#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    use windows::Win32::{
        Foundation::{HWND, RECT},
        UI::WindowsAndMessaging::{GetWindowRect, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos},
    };
    let raw = std::env::args().nth(1).unwrap().parse::<isize>().unwrap();
    let hwnd = HWND(raw as *mut _);
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut rect)? };
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
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOACTIVATE | SWP_NOZORDER,
        )?
    };
    Ok(())
}
#[cfg(not(windows))]
fn main() {}
