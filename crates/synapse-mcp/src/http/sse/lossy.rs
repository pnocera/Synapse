use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub(super) fn record_dropped(dropped_total: &AtomicU64, lossy_pending: &AtomicBool, count: u64) {
    if count == 0 {
        return;
    }
    dropped_total.fetch_add(count, Ordering::AcqRel);
    lossy_pending.store(true, Ordering::Release);
}

pub(super) fn record_lossy(lossy_pending: &AtomicBool) {
    lossy_pending.store(true, Ordering::Release);
}

pub(super) fn take_pending(lossy_pending: &AtomicBool) -> bool {
    lossy_pending.swap(false, Ordering::AcqRel)
}

pub(super) fn pending(lossy_pending: &AtomicBool) -> bool {
    lossy_pending.load(Ordering::Acquire)
}
