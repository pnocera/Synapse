use synapse_core::{AccessibleNode, AccessibleSubtree, UiaPattern};

use crate::{A11yResult, UIElement, platform};

/// Captures a UIA subtree rooted at `root`.
///
/// # Errors
///
/// Returns `A11Y_ELEMENT_STALE` when the root no longer produces a node, a
/// structured UIA error for OS failures, or `A11Y_NOT_AVAILABLE` on
/// non-Windows platforms.
pub fn snapshot(root: &UIElement, depth: u32) -> A11yResult<AccessibleSubtree> {
    platform::snapshot(root, depth)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ElementSearchScope {
    Children,
    Descendants,
    Subtree,
}

/// Finds the first enabled element under `root` with the requested UIA name and
/// pattern availability. This uses direct UIA search with a `RawView` cache.
///
/// # Errors
///
/// Returns a structured UIA error for OS failures, or `A11Y_NOT_AVAILABLE` on
/// non-Windows platforms.
pub fn find_by_name_and_pattern(
    root: &UIElement,
    name: &str,
    pattern: UiaPattern,
    scope: ElementSearchScope,
) -> A11yResult<Option<AccessibleNode>> {
    platform::find_by_name_and_pattern(root, name, pattern, scope)
}
