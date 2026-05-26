#[cfg(windows)]
pub use uiautomation;
#[cfg(windows)]
pub use uiautomation::UIElement;

#[cfg(not(windows))]
#[derive(Clone, Debug)]
pub struct UIElement;
