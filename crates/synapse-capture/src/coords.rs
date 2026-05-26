use synapse_core::Point;

use crate::{CaptureError, controller::validate_hwnd, platform};

/// Converts a screen-coordinate point to client/window coordinates.
///
/// # Errors
///
/// Returns [`CaptureError`] when the HWND is invalid or the OS coordinate
/// conversion fails.
pub fn screen_to_window(point: Point, hwnd: i64) -> Result<Point, CaptureError> {
    validate_hwnd(hwnd)?;
    platform::screen_to_window_impl(point, hwnd)
}

/// Converts a client/window-coordinate point to screen coordinates.
///
/// # Errors
///
/// Returns [`CaptureError`] when the HWND is invalid or the OS coordinate
/// conversion fails.
pub fn window_to_screen(point: Point, hwnd: i64) -> Result<Point, CaptureError> {
    validate_hwnd(hwnd)?;
    platform::window_to_screen_impl(point, hwnd)
}

#[must_use]
pub const fn screen_to_window_with_origin(point: Point, window_origin: Point) -> Point {
    Point {
        x: point.x - window_origin.x,
        y: point.y - window_origin.y,
    }
}

#[must_use]
pub const fn window_to_screen_with_origin(point: Point, window_origin: Point) -> Point {
    Point {
        x: point.x + window_origin.x,
        y: point.y + window_origin.y,
    }
}
