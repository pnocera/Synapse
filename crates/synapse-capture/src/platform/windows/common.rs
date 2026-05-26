use std::ffi::c_void;

use windows::Win32::Foundation::HWND;

use crate::CaptureError;

pub(super) fn capture_unsupported<E: std::fmt::Display>(err: E) -> CaptureError {
    CaptureError::GraphicsApiUnsupported {
        detail: err.to_string(),
    }
}

#[allow(clippy::missing_const_for_fn)]
pub(super) fn hwnd_from_i64(hwnd: i64) -> HWND {
    HWND(hwnd as *mut c_void)
}
