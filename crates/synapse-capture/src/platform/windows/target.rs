use windows::Win32::UI::WindowsAndMessaging::IsWindow;

use crate::CaptureError;

use super::common::hwnd_from_i64;

pub fn validate_hwnd(hwnd: i64) -> Result<(), CaptureError> {
    let hwnd = hwnd_from_i64(hwnd);
    if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        Ok(())
    } else {
        Err(CaptureError::TargetInvalid {
            detail: "HWND is not a live window".to_owned(),
        })
    }
}
