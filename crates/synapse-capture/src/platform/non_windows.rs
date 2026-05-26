use std::{sync::atomic::Ordering, thread, time::Duration};

use synapse_core::Point;

use crate::{
    CaptureConfig, CaptureError, CaptureThreadPriority, CapturedFrame, DpiAwarenessStatus,
    controller::{CaptureThreadContext, push_frame},
};

#[cfg(not(windows))]
#[allow(clippy::needless_pass_by_value)]
pub fn run_graphics_capture(
    config: CaptureConfig,
    ctx: CaptureThreadContext,
) -> Result<(), CaptureError> {
    run_synthetic_capture_loop(&config, &ctx)
}

#[cfg(not(windows))]
#[allow(clippy::needless_pass_by_value)]
pub fn run_dxgi_capture(
    config: CaptureConfig,
    ctx: CaptureThreadContext,
) -> Result<(), CaptureError> {
    run_synthetic_capture_loop(&config, &ctx)
}

#[cfg(not(windows))]
fn run_synthetic_capture_loop(
    config: &CaptureConfig,
    ctx: &CaptureThreadContext,
) -> Result<(), CaptureError> {
    let interval = Duration::from_millis(config.min_update_interval_ms.max(1));
    let mut frame_seq = 0_u64;
    while !ctx.stop.load(Ordering::Relaxed) {
        push_frame(ctx, CapturedFrame::synthetic(frame_seq, 1920, 1080))?;
        frame_seq = frame_seq.saturating_add(1);
        thread::sleep(interval);
    }
    Ok(())
}

#[cfg(not(windows))]
#[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
pub fn screen_to_window_impl(point: Point, _hwnd: i64) -> Result<Point, CaptureError> {
    Ok(point)
}

#[cfg(not(windows))]
#[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
pub fn window_to_screen_impl(point: Point, _hwnd: i64) -> Result<Point, CaptureError> {
    Ok(point)
}

#[cfg(not(windows))]
#[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
pub fn init_process_dpi_awareness_impl() -> Result<DpiAwarenessStatus, CaptureError> {
    Ok(DpiAwarenessStatus::Unsupported)
}

#[cfg(not(windows))]
pub const fn is_per_monitor_v2_dpi_aware_impl() -> bool {
    false
}

#[cfg(not(windows))]
pub const fn current_thread_priority_impl() -> CaptureThreadPriority {
    CaptureThreadPriority::Unsupported
}

#[cfg(not(windows))]
#[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
pub fn set_capture_thread_priority() -> Result<(), CaptureError> {
    Ok(())
}

#[cfg(not(windows))]
#[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
pub fn validate_hwnd_impl(_hwnd: i64) -> Result<(), CaptureError> {
    Ok(())
}
