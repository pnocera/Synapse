#[cfg(any(test, windows))]
use std::fmt::Display;

#[cfg(any(test, windows))]
use synapse_core::{Action, AimCurve, AimNaturalParams, Backend, ButtonAction, MouseTarget};
use synapse_core::{ElementId, MouseButton, Point};

use crate::{ActionBackend, ActionError, ActionResult, EmitState};

#[cfg(windows)]
use synapse_a11y::{
    UIElement,
    uiautomation::{patterns::UIInvokePattern, types::Rect as UiaRect},
};

#[cfg(any(test, windows))]
const FALLBACK_MOVE_DURATION_MS: u32 = 50;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ElementClickOutcome {
    Invoked,
    CoordinateFallback(CoordinateFallbackPlan),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct CoordinateFallbackPlan {
    pub screen_point: Point,
    pub window_point: Point,
}

/// Re-resolves a Synapse accessibility element and invokes its UIA
/// `InvokePattern` without moving the cursor.
///
/// # Errors
///
/// On Windows, `synapse_a11y::re_resolve` failures are mapped to
/// `ACTION_ELEMENT_NOT_RESOLVED`. Missing or failing `InvokePattern` calls are
/// reported as `ACTION_TARGET_INVALID` so the higher-level click path can fall
/// through to coordinate click handling.
#[cfg(windows)]
pub fn invoke_element(element_id: &ElementId) -> ActionResult<()> {
    let element = synapse_a11y::re_resolve(element_id).map_err(element_not_resolved)?;
    invoke_resolved_element(element_id, &element)
}

/// Non-Windows builds expose the same API but fail closed before any action is
/// attempted.
///
/// # Errors
///
/// Always returns `ACTION_BACKEND_UNAVAILABLE` because UI Automation
/// `InvokePattern` dispatch is Windows-only.
#[cfg(not(windows))]
pub fn invoke_element(element_id: &ElementId) -> ActionResult<()> {
    Err(ActionError::BackendUnavailable {
        detail: format!("UI Automation InvokePattern requires Windows for element {element_id}"),
    })
}

/// Attempts semantic UIA invoke first, then falls back to a coordinate click at
/// the resolved element's bounding-rectangle center when `InvokePattern` is not
/// available.
///
/// # Errors
///
/// Returns `ACTION_ELEMENT_NOT_RESOLVED` when UIA re-resolution fails,
/// `ACTION_TARGET_INVALID` when the element cannot produce a usable click
/// target, or a backend-specific action error if the coordinate click cannot be
/// emitted.
#[cfg(windows)]
pub fn click_element_or_fallback<B>(
    element_id: &ElementId,
    backend: &B,
    state: &mut EmitState,
    button: MouseButton,
) -> ActionResult<ElementClickOutcome>
where
    B: ActionBackend,
{
    let element = synapse_a11y::re_resolve(element_id).map_err(element_not_resolved)?;

    complete_click_attempt(
        try_invoke_resolved_element(element_id, &element),
        || coordinate_fallback_plan(element_id, &element),
        backend,
        state,
        button,
    )
}

/// Non-Windows builds expose the same API but fail closed before any action is
/// attempted.
///
/// # Errors
///
/// Always returns `ACTION_BACKEND_UNAVAILABLE` because UI Automation
/// `InvokePattern` dispatch and bounding-rectangle fallback are Windows-only.
#[cfg(not(windows))]
pub fn click_element_or_fallback<B>(
    element_id: &ElementId,
    _backend: &B,
    _state: &mut EmitState,
    _button: MouseButton,
) -> ActionResult<ElementClickOutcome>
where
    B: ActionBackend,
{
    Err(ActionError::BackendUnavailable {
        detail: format!("UI Automation element click requires Windows for element {element_id}"),
    })
}

#[cfg(windows)]
fn invoke_resolved_element(element_id: &ElementId, element: &UIElement) -> ActionResult<()> {
    match try_invoke_resolved_element(element_id, element) {
        Ok(()) => Ok(()),
        Err(InvokeAttemptError::MissingPattern) => Err(invoke_pattern_unavailable(
            element_id,
            "pattern not available",
        )),
        Err(InvokeAttemptError::InvokeFailed(error)) => Err(error),
    }
}

#[cfg(windows)]
fn try_invoke_resolved_element(
    element_id: &ElementId,
    element: &UIElement,
) -> Result<(), InvokeAttemptError> {
    let pattern: UIInvokePattern = element
        .get_pattern()
        .map_err(|_err| InvokeAttemptError::MissingPattern)?;

    pattern
        .invoke()
        .map_err(|err| InvokeAttemptError::InvokeFailed(invoke_pattern_failed(element_id, err)))
}

#[cfg(any(test, windows))]
fn complete_click_attempt<B, F>(
    invoke_attempt: Result<(), InvokeAttemptError>,
    fallback_plan: F,
    backend: &B,
    state: &mut EmitState,
    button: MouseButton,
) -> ActionResult<ElementClickOutcome>
where
    B: ActionBackend,
    F: FnOnce() -> ActionResult<CoordinateFallbackPlan>,
{
    match invoke_attempt {
        Ok(()) => Ok(ElementClickOutcome::Invoked),
        Err(InvokeAttemptError::MissingPattern) => {
            let plan = fallback_plan()?;
            emit_coordinate_fallback_click(backend, state, button, plan)?;
            Ok(ElementClickOutcome::CoordinateFallback(plan))
        }
        Err(InvokeAttemptError::InvokeFailed(error)) => Err(error),
    }
}

#[cfg(any(test, windows))]
enum InvokeAttemptError {
    MissingPattern,
    InvokeFailed(ActionError),
}

#[cfg(windows)]
fn coordinate_fallback_plan(
    element_id: &ElementId,
    element: &UIElement,
) -> ActionResult<CoordinateFallbackPlan> {
    let parts = element_id.parts().map_err(target_invalid)?;
    let rect = element.get_bounding_rectangle().map_err(|err| {
        target_invalid(format!(
            "element {element_id} bounding rectangle unavailable: {err}"
        ))
    })?;
    let screen_point = center_from_rect_edges(RectEdges::from(rect))?;
    let window_point =
        synapse_capture::screen_to_window(screen_point, parts.hwnd).map_err(|err| {
            target_invalid(format!(
                "element {element_id} screen_to_window failed for {screen_point:?}: {err}"
            ))
        })?;

    Ok(CoordinateFallbackPlan {
        screen_point,
        window_point,
    })
}

#[cfg(any(test, windows))]
fn emit_coordinate_fallback_click<B>(
    backend: &B,
    state: &mut EmitState,
    button: MouseButton,
    plan: CoordinateFallbackPlan,
) -> ActionResult<()>
where
    B: ActionBackend,
{
    backend.execute(
        &Action::MouseMove {
            to: MouseTarget::Screen {
                point: plan.screen_point,
            },
            curve: AimCurve::Natural {
                params: AimNaturalParams::FAST,
            },
            duration_ms: FALLBACK_MOVE_DURATION_MS,
            backend: Backend::Software,
        },
        state,
    )?;
    backend.execute(
        &Action::MouseButton {
            button,
            action: ButtonAction::Down,
            hold_ms: 0,
            backend: Backend::Software,
        },
        state,
    )?;
    backend.execute(
        &Action::MouseButton {
            button,
            action: ButtonAction::Up,
            hold_ms: 0,
            backend: Backend::Software,
        },
        state,
    )
}

#[must_use]
#[cfg(any(test, windows))]
fn element_not_resolved(error: impl Display) -> ActionError {
    ActionError::ElementNotResolved {
        detail: error.to_string(),
    }
}

#[must_use]
#[cfg(any(test, windows))]
fn invoke_pattern_unavailable(element_id: &ElementId, error: impl Display) -> ActionError {
    ActionError::TargetInvalid {
        detail: format!("element {element_id} does not expose InvokePattern: {error}"),
    }
}

#[must_use]
#[cfg(any(test, windows))]
fn invoke_pattern_failed(element_id: &ElementId, error: impl Display) -> ActionError {
    ActionError::TargetInvalid {
        detail: format!("InvokePattern.invoke failed for element {element_id}: {error}"),
    }
}

#[must_use]
#[cfg(any(test, windows))]
fn target_invalid(error: impl Display) -> ActionError {
    ActionError::TargetInvalid {
        detail: error.to_string(),
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[cfg(any(test, windows))]
struct RectEdges {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[cfg(windows)]
impl From<UiaRect> for RectEdges {
    fn from(value: UiaRect) -> Self {
        Self {
            left: value.get_left(),
            top: value.get_top(),
            right: value.get_right(),
            bottom: value.get_bottom(),
        }
    }
}

#[cfg(any(test, windows))]
fn center_from_rect_edges(rect: RectEdges) -> ActionResult<Point> {
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return Err(ActionError::TargetInvalid {
            detail: format!("element bounding rectangle is empty or inverted: {rect:?}"),
        });
    }

    let width = i64::from(rect.right) - i64::from(rect.left);
    let height = i64::from(rect.bottom) - i64::from(rect.top);
    let x = i64::from(rect.left) + width / 2;
    let y = i64::from(rect.top) + height / 2;

    Ok(Point {
        x: i32::try_from(x).map_err(target_invalid)?,
        y: i32::try_from(y).map_err(target_invalid)?,
    })
}

#[cfg(test)]
mod tests {
    use synapse_core::{
        AimCurve, AimNaturalParams, ElementId, MouseButton, MouseTarget, Point, error_codes,
    };

    use super::{
        CoordinateFallbackPlan, ElementClickOutcome, InvokeAttemptError, RectEdges,
        center_from_rect_edges, complete_click_attempt, element_not_resolved,
        emit_coordinate_fallback_click, invoke_pattern_failed, invoke_pattern_unavailable,
    };
    #[cfg(not(windows))]
    use super::{click_element_or_fallback, invoke_element};
    use crate::ActionError;
    use crate::{EmitState, RecordedInput, RecordingBackend};

    #[test]
    fn re_resolve_failures_map_to_element_not_resolved() {
        let before = "synthetic stale element";
        let after = element_not_resolved(before);
        assert_eq!(after.code(), error_codes::ACTION_ELEMENT_NOT_RESOLVED);
        assert_eq!(after.detail(), before);
        println!(
            "source_of_truth=invoke_error_mapping edge=re_resolve_failure before={before:?} after_code={} after_detail={:?}",
            after.code(),
            after.detail()
        );
    }

    #[test]
    fn missing_invoke_pattern_maps_to_target_invalid_for_coordinate_fallback() {
        let element_id = synthetic_element_id();
        let before = "pattern not available";
        let after = invoke_pattern_unavailable(&element_id, before);
        assert_eq!(after.code(), error_codes::ACTION_TARGET_INVALID);
        assert!(after.detail().contains(element_id.as_str()));
        assert!(after.detail().contains("InvokePattern"));
        println!(
            "source_of_truth=invoke_error_mapping edge=missing_invoke_pattern before={before:?} after_code={} after_detail={:?}",
            after.code(),
            after.detail()
        );
    }

    #[test]
    fn invoke_failures_map_to_target_invalid_without_cursor_fallback_in_bridge() {
        let element_id = synthetic_element_id();
        let before = "blocked by modal";
        let after = invoke_pattern_failed(&element_id, before);
        assert_eq!(after.code(), error_codes::ACTION_TARGET_INVALID);
        assert!(after.detail().contains(element_id.as_str()));
        assert!(after.detail().contains("InvokePattern.invoke failed"));
        println!(
            "source_of_truth=invoke_error_mapping edge=invoke_failure before={before:?} after_code={} after_detail={:?}",
            after.code(),
            after.detail()
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_stub_fails_closed() {
        let element_id = synthetic_element_id();
        let before = format!("os={} element_id={element_id}", std::env::consts::OS);
        let after = invoke_element(&element_id);
        let Err(ActionError::BackendUnavailable { detail }) = after else {
            panic!("expected non-Windows invoke_element to fail closed");
        };
        assert_eq!(
            ActionError::BackendUnavailable {
                detail: detail.clone()
            }
            .code(),
            error_codes::ACTION_BACKEND_UNAVAILABLE
        );
        assert!(detail.contains("requires Windows"));
        println!(
            "source_of_truth=invoke_non_windows_stub edge=non_windows before={before:?} after_code={} after_detail={detail:?}",
            error_codes::ACTION_BACKEND_UNAVAILABLE
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_click_fallback_fails_closed() {
        let element_id = synthetic_element_id();
        let backend = RecordingBackend::default();
        let mut state = EmitState::default();
        let before = format!(
            "os={} element_id={element_id} events={:?}",
            std::env::consts::OS,
            backend.events()
        );
        let after = click_element_or_fallback(&element_id, &backend, &mut state, MouseButton::Left);
        let Err(ActionError::BackendUnavailable { detail }) = after else {
            panic!("expected non-Windows click_element_or_fallback to fail closed");
        };
        assert_eq!(
            ActionError::BackendUnavailable {
                detail: detail.clone()
            }
            .code(),
            error_codes::ACTION_BACKEND_UNAVAILABLE
        );
        assert!(backend.events().is_empty());
        println!(
            "source_of_truth=invoke_coordinate_fallback edge=non_windows before={before:?} after_code={} after_detail={detail:?} after_events={:?}",
            error_codes::ACTION_BACKEND_UNAVAILABLE,
            backend.events()
        );
    }

    #[test]
    fn coordinate_fallback_emits_move_down_up_at_bbox_center() {
        let backend = RecordingBackend::default();
        let mut state = EmitState::default();
        let plan = CoordinateFallbackPlan {
            screen_point: Point { x: 60, y: 120 },
            window_point: Point { x: 10, y: 20 },
        };
        let before = backend.events();

        if let Err(error) =
            emit_coordinate_fallback_click(&backend, &mut state, MouseButton::Left, plan)
        {
            panic!("recording backend should accept coordinate fallback: {error}");
        }

        let after = backend.events();
        let expected = vec![
            RecordedInput::MouseMove {
                to: MouseTarget::Screen {
                    point: plan.screen_point,
                },
                curve: AimCurve::Natural {
                    params: AimNaturalParams::FAST,
                },
                duration_ms: super::FALLBACK_MOVE_DURATION_MS,
            },
            RecordedInput::MouseButtonDown {
                button: MouseButton::Left,
            },
            RecordedInput::MouseButtonUp {
                button: MouseButton::Left,
            },
        ];
        assert_eq!(after, expected);
        println!(
            "source_of_truth=recording_backend edge=coordinate_fallback_sequence before={before:?} after={after:?}"
        );
    }

    #[test]
    fn missing_invoke_pattern_branch_emits_coordinate_fallback() {
        let backend = RecordingBackend::default();
        let mut state = EmitState::default();
        let plan = CoordinateFallbackPlan {
            screen_point: Point { x: 42, y: 84 },
            window_point: Point { x: 2, y: 4 },
        };
        let before = backend.events();

        let after = complete_click_attempt(
            Err(InvokeAttemptError::MissingPattern),
            || Ok(plan),
            &backend,
            &mut state,
            MouseButton::Right,
        );

        assert_eq!(after, Ok(ElementClickOutcome::CoordinateFallback(plan)));
        let events = backend.events();
        assert_eq!(
            events,
            vec![
                RecordedInput::MouseMove {
                    to: MouseTarget::Screen {
                        point: plan.screen_point,
                    },
                    curve: AimCurve::Natural {
                        params: AimNaturalParams::FAST,
                    },
                    duration_ms: super::FALLBACK_MOVE_DURATION_MS,
                },
                RecordedInput::MouseButtonDown {
                    button: MouseButton::Right,
                },
                RecordedInput::MouseButtonUp {
                    button: MouseButton::Right,
                },
            ]
        );
        println!(
            "source_of_truth=invoke_coordinate_fallback edge=missing_invoke_pattern before={before:?} after_outcome={after:?} after_events={events:?}"
        );
    }

    #[test]
    fn successful_invoke_branch_does_not_emit_coordinate_fallback() {
        let backend = RecordingBackend::default();
        let mut state = EmitState::default();
        let before = backend.events();

        let after = complete_click_attempt(
            Ok(()),
            || {
                Ok(CoordinateFallbackPlan {
                    screen_point: Point { x: 1, y: 1 },
                    window_point: Point { x: 1, y: 1 },
                })
            },
            &backend,
            &mut state,
            MouseButton::Left,
        );

        assert_eq!(after, Ok(ElementClickOutcome::Invoked));
        assert!(backend.events().is_empty());
        println!(
            "source_of_truth=invoke_coordinate_fallback edge=invoke_success before={before:?} after_outcome={after:?} after_events={:?}",
            backend.events()
        );
    }

    #[test]
    fn failed_invoke_branch_does_not_emit_coordinate_fallback() {
        let backend = RecordingBackend::default();
        let mut state = EmitState::default();
        let expected_error = ActionError::TargetInvalid {
            detail: "synthetic invoke failure".to_owned(),
        };
        let before = backend.events();

        let after = complete_click_attempt(
            Err(InvokeAttemptError::InvokeFailed(expected_error.clone())),
            || {
                Ok(CoordinateFallbackPlan {
                    screen_point: Point { x: 99, y: 99 },
                    window_point: Point { x: 9, y: 9 },
                })
            },
            &backend,
            &mut state,
            MouseButton::Left,
        );

        assert_eq!(after, Err(expected_error));
        assert!(backend.events().is_empty());
        println!(
            "source_of_truth=invoke_coordinate_fallback edge=invoke_failure before={before:?} after_outcome={after:?} after_events={:?}",
            backend.events()
        );
    }

    #[test]
    fn bbox_center_rounds_inside_odd_sized_rectangle() {
        let rect = RectEdges {
            left: 10,
            top: 20,
            right: 111,
            bottom: 221,
        };
        let before = format!("{rect:?}");
        let after = match center_from_rect_edges(rect) {
            Ok(point) => point,
            Err(error) => panic!("odd-sized rect should have a center: {error}"),
        };
        let expected_exact_center = (60.5_f64, 120.5_f64);
        let dx = f64::from(after.x) - expected_exact_center.0;
        let dy = f64::from(after.y) - expected_exact_center.1;
        assert!(after.x >= rect.left && after.x < rect.right);
        assert!(after.y >= rect.top && after.y < rect.bottom);
        assert!(dx.hypot(dy) <= 1.0);
        println!(
            "source_of_truth=bbox_center edge=odd_sized before={before:?} after={after:?} expected_exact_center={expected_exact_center:?}"
        );
    }

    #[test]
    fn bbox_center_rejects_empty_or_inverted_rectangles() {
        for rect in [
            RectEdges {
                left: 5,
                top: 5,
                right: 5,
                bottom: 10,
            },
            RectEdges {
                left: 10,
                top: 10,
                right: 9,
                bottom: 12,
            },
            RectEdges {
                left: 10,
                top: 10,
                right: 12,
                bottom: 10,
            },
        ] {
            let before = format!("{rect:?}");
            let after = center_from_rect_edges(rect);
            let Err(error) = after else {
                panic!("expected invalid rect to fail: {rect:?}");
            };
            assert_eq!(error.code(), error_codes::ACTION_TARGET_INVALID);
            println!(
                "source_of_truth=bbox_center edge=invalid_rect before={before:?} after_code={} after_detail={:?}",
                error.code(),
                error.detail()
            );
        }
    }

    #[test]
    fn bbox_center_handles_large_screen_coordinates_without_overflow() {
        let rect = RectEdges {
            left: i32::MAX - 100,
            top: i32::MAX - 200,
            right: i32::MAX,
            bottom: i32::MAX - 20,
        };
        let before = format!("{rect:?}");
        let after = match center_from_rect_edges(rect) {
            Ok(point) => point,
            Err(error) => panic!("large rect should stay in i32 bounds: {error}"),
        };
        assert_eq!(
            after,
            Point {
                x: i32::MAX - 50,
                y: i32::MAX - 110,
            }
        );
        println!(
            "source_of_truth=bbox_center edge=large_coordinates before={before:?} after={after:?}"
        );
    }

    fn synthetic_element_id() -> ElementId {
        match ElementId::parse("0x1234:0000002a00000001") {
            Ok(element_id) => element_id,
            Err(error) => panic!("synthetic element id must parse: {error}"),
        }
    }
}
