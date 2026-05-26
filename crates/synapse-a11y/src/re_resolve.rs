use serde::{Deserialize, Serialize};
use synapse_core::ElementId;

use crate::{A11yResult, UIElement, platform};

/// Re-resolves a composite Synapse element id back to a live UIA element.
///
/// # Errors
///
/// Returns `A11Y_ELEMENT_STALE` when the runtime id cannot be found under the
/// HWND, `OBSERVE_INTERNAL` for invalid ids, or `A11Y_NOT_AVAILABLE` on
/// non-Windows platforms.
pub fn re_resolve(id: &ElementId) -> A11yResult<UIElement> {
    platform::re_resolve(id)
}

/// Read-only mirror of `uiautomation::types::ExpandCollapseState`. Kept
/// independent of the underlying crate so callers don't need a uiautomation
/// dependency just to compare against a literal.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpandState {
    Collapsed,
    Expanded,
    PartiallyExpanded,
    LeafNode,
}

/// Reads `ExpandCollapsePattern::CurrentExpandCollapseState` from the given
/// element.
///
/// Used by `act_click(use_invoke_pattern=true)` manual verification tests to
/// assert menu/expander state flipped after an invoke.
///
/// # Errors
///
/// Returns the same structured UIA errors as the other a11y accessors;
/// `A11Y_PATTERN_UNAVAILABLE`-class error when the element does not expose
/// `ExpandCollapsePattern`; `A11Y_NOT_AVAILABLE` on non-Windows platforms.
pub fn expand_state_of(element: &UIElement) -> A11yResult<ExpandState> {
    platform::expand_state_of(element)
}
