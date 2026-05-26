use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};

use crate::FRAMES_DROPPED_METRIC;

const THREAD_PRIORITY_UNKNOWN: i32 = i32::MIN;
const THREAD_PRIORITY_UNSUPPORTED: i32 = i32::MIN + 1;
const THREAD_PRIORITY_TIME_CRITICAL: i32 = i32::MAX;

#[derive(Debug)]
pub struct CaptureStats {
    frames_captured: AtomicU64,
    frames_dropped: AtomicU64,
    thread_priority: AtomicI32,
}

impl Default for CaptureStats {
    fn default() -> Self {
        Self {
            frames_captured: AtomicU64::new(0),
            frames_dropped: AtomicU64::new(0),
            thread_priority: AtomicI32::new(THREAD_PRIORITY_UNKNOWN),
        }
    }
}

impl CaptureStats {
    #[must_use]
    pub fn frames_captured(&self) -> u64 {
        self.frames_captured.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn frames_dropped(&self) -> u64 {
        self.frames_dropped.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn thread_priority(&self) -> CaptureThreadPriority {
        decode_thread_priority(self.thread_priority.load(Ordering::Relaxed))
    }

    pub(crate) fn increment_captured(&self) {
        self.frames_captured.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn increment_dropped(&self) {
        self.frames_dropped.fetch_add(1, Ordering::Relaxed);
        synapse_telemetry::metrics::counter!(FRAMES_DROPPED_METRIC).increment(1);
    }

    pub(crate) fn set_thread_priority(&self, priority: CaptureThreadPriority) {
        self.thread_priority
            .store(encode_thread_priority(priority), Ordering::Relaxed);
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CaptureThreadPriority {
    TimeCritical,
    Other(i32),
    Unsupported,
    Unknown,
}

const fn encode_thread_priority(priority: CaptureThreadPriority) -> i32 {
    match priority {
        CaptureThreadPriority::TimeCritical => THREAD_PRIORITY_TIME_CRITICAL,
        CaptureThreadPriority::Unsupported => THREAD_PRIORITY_UNSUPPORTED,
        CaptureThreadPriority::Unknown => THREAD_PRIORITY_UNKNOWN,
        CaptureThreadPriority::Other(value) => value,
    }
}

const fn decode_thread_priority(value: i32) -> CaptureThreadPriority {
    match value {
        THREAD_PRIORITY_TIME_CRITICAL => CaptureThreadPriority::TimeCritical,
        THREAD_PRIORITY_UNSUPPORTED => CaptureThreadPriority::Unsupported,
        THREAD_PRIORITY_UNKNOWN => CaptureThreadPriority::Unknown,
        other => CaptureThreadPriority::Other(other),
    }
}
