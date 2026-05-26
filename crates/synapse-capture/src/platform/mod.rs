#[cfg(not(windows))]
mod non_windows;
#[cfg(windows)]
mod windows;

#[cfg(not(windows))]
pub use non_windows::*;
#[cfg(windows)]
pub use windows::*;
