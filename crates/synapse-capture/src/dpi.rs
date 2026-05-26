use crate::{CaptureError, CaptureThreadPriority, platform};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DpiAwarenessStatus {
    Set,
    AlreadySet,
    Unsupported,
}
/// Initializes per-monitor-v2 DPI awareness for accurate physical-pixel math.
///
/// # Errors
///
/// Returns [`CaptureError`] when Windows rejects the DPI-awareness call for a
/// reason other than "already set".
pub fn init_process_dpi_awareness() -> Result<DpiAwarenessStatus, CaptureError> {
    platform::init_process_dpi_awareness_impl()
}

#[must_use]
#[allow(clippy::missing_const_for_fn)]
pub fn is_per_monitor_v2_dpi_aware() -> bool {
    platform::is_per_monitor_v2_dpi_aware_impl()
}

#[must_use]
#[allow(clippy::missing_const_for_fn)]
pub fn current_thread_priority() -> CaptureThreadPriority {
    platform::current_thread_priority_impl()
}
