#![allow(unsafe_code)]

mod backend;
mod bitmap;
mod config;
mod controller;
mod coords;
mod dpi;
mod error;
mod frame;
mod platform;
mod stats;

pub use backend::{CaptureBackend, CaptureBackendPreference};
pub use bitmap::*;
pub use config::{CaptureConfig, CaptureTarget, ResolvedCaptureTarget};
pub use controller::{
    CaptureController, CaptureHandle, register_capture_metrics, resolve_capture_target,
    spawn_capture_loop,
};
pub use coords::*;
pub use dpi::*;
pub use error::*;
pub use frame::*;
pub use stats::{CaptureStats, CaptureThreadPriority};

#[cfg(test)]
pub(crate) use backend::{backend_after_fallback, should_fallback_to_dxgi};

pub const CAPTURE_CHANNEL_CAPACITY: usize = 2;
pub const FRAMES_DROPPED_METRIC: &str = "synapse_capture_frames_dropped_total";

#[cfg(test)]
mod tests;
