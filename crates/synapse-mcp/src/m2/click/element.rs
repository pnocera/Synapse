use std::time::Instant;

use rmcp::ErrorData;
use synapse_action::{
    ActionError, ActionHandle, DoubleClickTiming, ElementClickOutcome, EmitState, RecordingBackend,
    click_element_or_fallback,
};
use synapse_core::{
    Action, ButtonAction, MouseButton, MouseTarget, Point, UiaPattern, error_codes,
};
use tokio::time::{Duration, sleep};

#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, POINT as WinPoint, RECT, WPARAM},
        Graphics::Gdi::ScreenToClient,
        UI::WindowsAndMessaging::{
            EnumChildWindows, GetClassNameW, GetWindowRect, IsWindow, IsWindowVisible,
            PostMessageW, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE,
            WM_RBUTTONDOWN, WM_RBUTTONUP,
        },
    },
    core::BOOL,
};

use super::{
    CLICK_REASON_BACKEND_UNAVAILABLE, CLICK_TIER_FOREGROUND, CLICK_TIER_POSTMESSAGE,
    CLICK_TIER_UIA, action_error_to_mcp, attach_click_tier_attempts, backend_used_name,
    click_backend_tier_used, click_error_code, click_reason_for_error_code,
    click_required_foreground, click_tier_delivered, click_tier_failed,
    error_has_click_tier_attempts, record,
    schema::{
        ActClickElementTarget, ActClickParams, ActClickResponse, ActClickTierAttempt,
        postcondition_not_requested,
    },
};

pub(super) async fn execute_element_click(
    handle: ActionHandle,
    params: &ActClickParams,
    element: &ActClickElementTarget,
    recording: Option<&RecordingBackend>,
    timing: DoubleClickTiming,
    started: Instant,
) -> Result<ActClickResponse, ErrorData> {
    if element_is_coordinate_only(&element.element_id) || !params.use_invoke_pattern {
        return execute_coordinate_element_click(
            handle,
            params,
            element,
            recording,
            timing,
            started,
            Vec::new(),
            "coordinate_direct",
        )
        .await;
    }

    let mut state = EmitState::new();
    let mut used_invoke_pattern = false;
    let mut backend_used = "software";
    let mut uia_outcomes = Vec::new();
    for click_index in 0..params.clicks {
        let outcome_result = if let Some(recording) = recording {
            click_element_or_fallback(&element.element_id, recording, &mut state, params.button)
        } else {
            let backend = synapse_action::backend::software::SoftwareBackend::new();
            click_element_or_fallback(&element.element_id, &backend, &mut state, params.button)
        };
        let outcome = match outcome_result {
            Ok(outcome) => outcome,
            Err(error) => {
                let error_code = error.code().to_owned();
                let reason_code = click_reason_for_error_code(&error_code);
                let tier_attempts = vec![click_tier_failed(
                    CLICK_TIER_UIA,
                    reason_code,
                    error_code.clone(),
                    false,
                    error.detail().to_owned(),
                )];
                if error_code == error_codes::ACTION_ELEMENT_PATTERN_UNSUPPORTED
                    && params.coordinate_fallback_on_unsupported
                {
                    let metadata = element_coordinate_fallback_metadata(&element.element_id)?;
                    if coordinate_fallback_allowed_for_metadata(&metadata) {
                        tracing::info!(
                            code = "M2_ACT_CLICK_COORDINATE_FALLBACK_ON_UNSUPPORTED",
                            kind = "act_click",
                            element_id = %element.element_id,
                            role = %metadata.role,
                            automation_id = metadata.automation_id.as_deref(),
                            enabled = metadata.enabled,
                            keyboard_focusable = metadata.keyboard_focusable,
                            patterns = ?metadata.patterns,
                            bbox = ?metadata.bbox,
                            "act_click UIA pattern unsupported; explicit coordinate fallback allowed for focusable edit/document-like element"
                        );
                        return execute_coordinate_element_click(
                            handle,
                            params,
                            element,
                            recording,
                            timing,
                            started,
                            tier_attempts,
                            "coordinate_fallback_on_unsupported",
                        )
                        .await;
                    }
                    tracing::warn!(
                        code = "M2_ACT_CLICK_COORDINATE_FALLBACK_DENIED",
                        kind = "act_click",
                        element_id = %element.element_id,
                        role = %metadata.role,
                        automation_id = metadata.automation_id.as_deref(),
                        enabled = metadata.enabled,
                        keyboard_focusable = metadata.keyboard_focusable,
                        patterns = ?metadata.patterns,
                        bbox = ?metadata.bbox,
                        "act_click UIA pattern unsupported; coordinate fallback denied because element is not an enabled focusable edit/document-like target"
                    );
                }
                let mcp_error = action_error_to_mcp(&error);
                if error_has_click_tier_attempts(&mcp_error) {
                    return Err(mcp_error);
                }
                return Err(attach_click_tier_attempts(mcp_error, tier_attempts));
            }
        };

        match outcome {
            ElementClickOutcome::Invoked => {
                trace_element_click_outcome(
                    element,
                    click_index,
                    "invoked",
                    ElementClickTraceReadback::default(),
                );
                used_invoke_pattern = true;
                backend_used = "uia";
                uia_outcomes.push("invoked".to_owned());
            }
            ElementClickOutcome::Toggled {
                before_state,
                after_state,
            } => {
                trace_element_click_outcome(
                    element,
                    click_index,
                    "toggled",
                    ElementClickTraceReadback {
                        state_before: Some(before_state.as_str()),
                        state_after: Some(after_state.as_str()),
                        ..ElementClickTraceReadback::default()
                    },
                );
                used_invoke_pattern = true;
                backend_used = "uia";
                uia_outcomes.push("toggled".to_owned());
            }
            ElementClickOutcome::Selected {
                was_selected,
                is_selected,
            } => {
                trace_element_click_outcome(
                    element,
                    click_index,
                    "selected",
                    ElementClickTraceReadback {
                        selected_before: Some(was_selected),
                        selected_after: Some(is_selected),
                        ..ElementClickTraceReadback::default()
                    },
                );
                used_invoke_pattern = true;
                backend_used = "uia";
                uia_outcomes.push("selected".to_owned());
            }
            ElementClickOutcome::Expanded {
                before_state,
                after_state,
            } => {
                let before_state = format!("{before_state:?}");
                let after_state = format!("{after_state:?}");
                trace_element_click_outcome(
                    element,
                    click_index,
                    "expanded",
                    ElementClickTraceReadback {
                        state_before: Some(before_state.as_str()),
                        state_after: Some(after_state.as_str()),
                        ..ElementClickTraceReadback::default()
                    },
                );
                used_invoke_pattern = true;
                backend_used = "uia";
                uia_outcomes.push("expanded".to_owned());
            }
            ElementClickOutcome::Collapsed {
                before_state,
                after_state,
            } => {
                let before_state = format!("{before_state:?}");
                let after_state = format!("{after_state:?}");
                trace_element_click_outcome(
                    element,
                    click_index,
                    "collapsed",
                    ElementClickTraceReadback {
                        state_before: Some(before_state.as_str()),
                        state_after: Some(after_state.as_str()),
                        ..ElementClickTraceReadback::default()
                    },
                );
                used_invoke_pattern = true;
                backend_used = "uia";
                uia_outcomes.push("collapsed".to_owned());
            }
            ElementClickOutcome::LegacyDefaultAction { default_action } => {
                trace_element_click_outcome(
                    element,
                    click_index,
                    "legacy_default_action",
                    ElementClickTraceReadback {
                        legacy_default_action: default_action.as_deref(),
                        ..ElementClickTraceReadback::default()
                    },
                );
                used_invoke_pattern = true;
                backend_used = "uia";
                uia_outcomes.push("legacy_default_action".to_owned());
            }
            ElementClickOutcome::CoordinateFallback(plan) => {
                tracing::error!(
                    code = "M2_ACT_CLICK_UNEXPECTED_COORDINATE_FALLBACK",
                    kind = "act_click",
                    element_id = %element.element_id,
                    screen_x = plan.screen_point.x,
                    screen_y = plan.screen_point.y,
                    "semantic UIA element click returned a coordinate fallback plan; no fallback delivery attempted"
                );
                return Err(action_error_to_mcp(
                    &ActionError::ElementPatternUnsupported {
                        element_id: element.element_id.clone(),
                        detail: format!(
                            "semantic UIA click path unexpectedly produced coordinate fallback plan {plan:?}; no fallback delivery was attempted"
                        ),
                    },
                ));
            }
        }

        if click_index + 1 < params.clicks {
            sleep(Duration::from_millis(u64::from(
                timing.inter_click_delay_ms,
            )))
            .await;
        }
    }

    let tier_attempts = vec![click_tier_delivered(
        CLICK_TIER_UIA,
        false,
        format!(
            "UI Automation semantic click delivered; outcomes={}",
            uia_outcomes.join(",")
        ),
    )];
    Ok(ActClickResponse {
        ok: true,
        used_invoke_pattern,
        backend_used: backend_used.to_owned(),
        backend_tier_used: click_backend_tier_used(&tier_attempts),
        required_foreground: click_required_foreground(&tier_attempts),
        tier_attempts,
        postcondition: postcondition_not_requested(),
        press_hold_ms: params.hold_ms,
        double_click_window_ms: timing.window_ms,
        inter_click_delay_ms: timing.inter_click_delay_ms,
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    })
}

async fn execute_coordinate_element_click(
    handle: ActionHandle,
    params: &ActClickParams,
    element: &ActClickElementTarget,
    recording: Option<&RecordingBackend>,
    timing: DoubleClickTiming,
    started: Instant,
    mut tier_attempts: Vec<ActClickTierAttempt>,
    trace_outcome: &'static str,
) -> Result<ActClickResponse, ErrorData> {
    let screen_point = match element_center(&element.element_id) {
        Ok(screen_point) => screen_point,
        Err(error) => {
            let error_code = click_error_code(&error);
            let reason_code = click_reason_for_error_code(&error_code);
            tier_attempts.push(click_tier_failed(
                CLICK_TIER_FOREGROUND,
                reason_code,
                error_code,
                true,
                error.message.to_string(),
            ));
            return Err(attach_click_tier_attempts(error, tier_attempts));
        }
    };
    trace_element_click_outcome(
        element,
        0,
        trace_outcome,
        ElementClickTraceReadback {
            fallback_screen_point: Some(screen_point),
            ..ElementClickTraceReadback::default()
        },
    );
    let actions = coordinate_click_actions(params, screen_point);
    let backend_used = if let Some(recording) = recording {
        if let Err(error) =
            record::execute_recording(recording, &actions, params.clicks, timing).await
        {
            let error_code = click_error_code(&error);
            let reason_code = click_reason_for_error_code(&error_code);
            tier_attempts.push(click_tier_failed(
                CLICK_TIER_FOREGROUND,
                reason_code,
                error_code,
                true,
                error.message.to_string(),
            ));
            return Err(attach_click_tier_attempts(error, tier_attempts));
        }
        tier_attempts.push(click_tier_delivered(
            CLICK_TIER_FOREGROUND,
            true,
            "coordinate element click recorded through the foreground input tier",
        ));
        backend_used_name(params.backend).to_owned()
    } else {
        match record::execute_actor_actions(handle, actions, timing).await {
            Ok(()) => {
                tier_attempts.push(click_tier_delivered(
                    CLICK_TIER_FOREGROUND,
                    true,
                    "coordinate element click delivered through the foreground input tier",
                ));
                backend_used_name(params.backend).to_owned()
            }
            Err(error) if should_try_hwnd_message_fallback(&error) => {
                let foreground_detail = error.message.to_string();
                tier_attempts.push(click_tier_failed(
                    CLICK_TIER_FOREGROUND,
                    CLICK_REASON_BACKEND_UNAVAILABLE,
                    error_codes::ACTION_BACKEND_UNAVAILABLE,
                    true,
                    foreground_detail,
                ));
                match post_element_window_message_click(params, element, screen_point, timing).await
                {
                    Ok(backend_used) => {
                        tier_attempts.push(click_tier_delivered(
                            CLICK_TIER_POSTMESSAGE,
                            false,
                            "coordinate element click delivered through HWND PostMessage",
                        ));
                        backend_used
                    }
                    Err(error) => {
                        let error_code = click_error_code(&error);
                        let reason_code = click_reason_for_error_code(&error_code);
                        tier_attempts.push(click_tier_failed(
                            CLICK_TIER_POSTMESSAGE,
                            reason_code,
                            error_code,
                            false,
                            error.message.to_string(),
                        ));
                        return Err(attach_click_tier_attempts(error, tier_attempts));
                    }
                }
            }
            Err(error) => {
                let error_code = click_error_code(&error);
                let reason_code = click_reason_for_error_code(&error_code);
                tier_attempts.push(click_tier_failed(
                    CLICK_TIER_FOREGROUND,
                    reason_code,
                    error_code,
                    true,
                    error.message.to_string(),
                ));
                return Err(attach_click_tier_attempts(error, tier_attempts));
            }
        }
    };
    let backend_tier_used = click_backend_tier_used(&tier_attempts);
    let required_foreground = click_required_foreground(&tier_attempts);
    Ok(ActClickResponse {
        ok: true,
        used_invoke_pattern: false,
        backend_used,
        backend_tier_used,
        required_foreground,
        tier_attempts,
        postcondition: postcondition_not_requested(),
        press_hold_ms: params.hold_ms,
        double_click_window_ms: timing.window_ms,
        inter_click_delay_ms: timing.inter_click_delay_ms,
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    })
}

fn element_coordinate_fallback_metadata(
    element_id: &synapse_core::ElementId,
) -> Result<synapse_a11y::ElementMetadataReadback, ErrorData> {
    synapse_a11y::element_metadata(element_id)
        .map_err(|error| action_error_to_mcp(&element_resolution_error(element_id, error)))
}

fn coordinate_fallback_allowed_for_metadata(
    metadata: &synapse_a11y::ElementMetadataReadback,
) -> bool {
    metadata.enabled
        && metadata.keyboard_focusable
        && metadata.bbox.w > 0
        && metadata.bbox.h > 0
        && (editable_role(&metadata.role) || exposes_text_value_pattern(&metadata.patterns))
}

fn editable_role(role: &str) -> bool {
    let role = role.to_ascii_lowercase();
    role.contains("edit") || role.contains("document") || role.contains("text")
}

fn exposes_text_value_pattern(patterns: &[UiaPattern]) -> bool {
    patterns
        .iter()
        .any(|pattern| matches!(pattern, UiaPattern::Value | UiaPattern::Text))
}

pub(super) async fn execute_element_postmessage_click(
    params: &ActClickParams,
    element: &ActClickElementTarget,
    mut tier_attempts: Vec<ActClickTierAttempt>,
    timing: DoubleClickTiming,
    started: Instant,
) -> Result<ActClickResponse, ErrorData> {
    let screen_point = match element_center(&element.element_id) {
        Ok(point) => point,
        Err(error) => {
            let error_code = click_error_code(&error);
            let reason_code = click_reason_for_error_code(&error_code);
            tier_attempts.push(click_tier_failed(
                CLICK_TIER_POSTMESSAGE,
                reason_code,
                error_code,
                false,
                error.message.to_string(),
            ));
            return Err(attach_click_tier_attempts(error, tier_attempts));
        }
    };
    let backend_used =
        match post_element_window_message_click(params, element, screen_point, timing).await {
            Ok(backend_used) => {
                tier_attempts.push(click_tier_delivered(
                    CLICK_TIER_POSTMESSAGE,
                    false,
                    "element click delivered through HWND PostMessage",
                ));
                backend_used
            }
            Err(error) => {
                let error_code = click_error_code(&error);
                let reason_code = click_reason_for_error_code(&error_code);
                tier_attempts.push(click_tier_failed(
                    CLICK_TIER_POSTMESSAGE,
                    reason_code,
                    error_code,
                    false,
                    error.message.to_string(),
                ));
                return Err(attach_click_tier_attempts(error, tier_attempts));
            }
        };

    Ok(ActClickResponse {
        ok: true,
        used_invoke_pattern: false,
        backend_used,
        backend_tier_used: click_backend_tier_used(&tier_attempts),
        required_foreground: click_required_foreground(&tier_attempts),
        tier_attempts,
        postcondition: postcondition_not_requested(),
        press_hold_ms: params.hold_ms,
        double_click_window_ms: timing.window_ms,
        inter_click_delay_ms: timing.inter_click_delay_ms,
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    })
}

fn coordinate_click_actions(params: &ActClickParams, screen_point: Point) -> Vec<Action> {
    let mut actions = Vec::with_capacity(usize::from(params.clicks) + 1);
    actions.push(Action::MouseMove {
        to: MouseTarget::Screen {
            point: screen_point,
        },
        curve: params.velocity_profile.to_aim_curve(),
        duration_ms: params.duration_ms,
        backend: params.backend,
    });
    for _ in 0..params.clicks {
        actions.push(Action::MouseButton {
            button: params.button,
            action: ButtonAction::Press,
            hold_ms: params.hold_ms,
            backend: params.backend,
        });
    }
    actions
}

fn should_try_hwnd_message_fallback(error: &ErrorData) -> bool {
    error
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .and_then(serde_json::Value::as_str)
        == Some(error_codes::ACTION_BACKEND_UNAVAILABLE)
}

#[cfg(windows)]
async fn post_element_window_message_click(
    params: &ActClickParams,
    element: &ActClickElementTarget,
    screen_point: Point,
    timing: DoubleClickTiming,
) -> Result<String, ErrorData> {
    let readback =
        windows_hwnd_message_click_readback(&element.element_id, screen_point, params.button)
            .map_err(|error| action_error_to_mcp(&error))?;
    for click_index in 0..params.clicks {
        post_mouse_message(readback.hwnd, WM_MOUSEMOVE, 0, readback.client_point)
            .map_err(|error| action_error_to_mcp(&error))?;
        let (down_message, up_message, down_wparam) =
            mouse_button_messages(params.button).map_err(|error| action_error_to_mcp(&error))?;
        post_mouse_message(
            readback.hwnd,
            down_message,
            down_wparam,
            readback.client_point,
        )
        .map_err(|error| action_error_to_mcp(&error))?;
        sleep(Duration::from_millis(u64::from(params.hold_ms))).await;
        post_mouse_message(readback.hwnd, up_message, 0, readback.client_point)
            .map_err(|error| action_error_to_mcp(&error))?;

        tracing::info!(
            code = "M2_ACT_CLICK_ELEMENT_HWND_MESSAGE_FALLBACK",
            kind = "act_click",
            element_id = %element.element_id,
            root_hwnd = readback.root_hwnd,
            target_hwnd = readback.hwnd,
            target_class = %readback.class_name,
            screen_x = screen_point.x,
            screen_y = screen_point.y,
            client_x = readback.client_point.x,
            client_y = readback.client_point.y,
            click_number = u32::from(click_index) + 1,
            button = ?params.button,
            "readback=window_message tool=act_click element_click_after"
        );

        if click_index + 1 < params.clicks {
            sleep(Duration::from_millis(u64::from(
                timing.inter_click_delay_ms,
            )))
            .await;
        }
    }
    Ok("software_window_message".to_owned())
}

#[cfg(not(windows))]
async fn post_element_window_message_click(
    _params: &ActClickParams,
    element: &ActClickElementTarget,
    _screen_point: Point,
    _timing: DoubleClickTiming,
) -> Result<String, ErrorData> {
    Err(action_error_to_mcp(&ActionError::BackendUnavailable {
        detail: format!(
            "act_click element target {} HWND message fallback requires Windows",
            element.element_id
        ),
    }))
}

#[cfg(windows)]
fn element_center(element_id: &synapse_core::ElementId) -> Result<Point, ErrorData> {
    let rect = if let Some(rect) = browser_ocr_rect_or_error(element_id)? {
        rect
    } else {
        synapse_a11y::element_bounding_rect(element_id)
            .map_err(|err| action_error_to_mcp(&element_resolution_error(element_id, err)))?
    };

    if rect.w <= 0 || rect.h <= 0 {
        return Err(action_error_to_mcp(&ActionError::TargetInvalid {
            detail: format!("act_click element bbox is empty or inverted: {rect:?}"),
        }));
    }

    let x = i64::from(rect.x) + i64::from(rect.w) / 2;
    let y = i64::from(rect.y) + i64::from(rect.h) / 2;
    Ok(Point {
        x: i32::try_from(x).map_err(|err| {
            action_error_to_mcp(&ActionError::TargetInvalid {
                detail: format!("act_click element bbox center x overflowed i32: {err}"),
            })
        })?,
        y: i32::try_from(y).map_err(|err| {
            action_error_to_mcp(&ActionError::TargetInvalid {
                detail: format!("act_click element bbox center y overflowed i32: {err}"),
            })
        })?,
    })
}

#[cfg(windows)]
fn element_resolution_error(
    element_id: &synapse_core::ElementId,
    error: synapse_a11y::A11yError,
) -> ActionError {
    let detail = error.to_string();
    match error {
        synapse_a11y::A11yError::ElementStale { .. } => ActionError::TransientElementExpired {
            element_id: element_id.clone(),
            detail,
        },
        _ => ActionError::ElementNotResolved {
            detail: format!("act_click element {element_id} could not be resolved: {detail}"),
        },
    }
}

#[cfg(windows)]
fn browser_ocr_rect_or_error(
    element_id: &synapse_core::ElementId,
) -> Result<Option<synapse_core::Rect>, ErrorData> {
    match crate::m1::browser_ocr_rect_from_element_id(element_id) {
        Some(rect) => Ok(Some(rect)),
        None if crate::m1::is_browser_ocr_element_id(element_id) => {
            Err(action_error_to_mcp(&ActionError::TargetInvalid {
                detail: format!(
                    "act_click browser OCR element {element_id} does not contain a valid non-empty bbox"
                ),
            }))
        }
        None => Ok(None),
    }
}

#[cfg(not(windows))]
fn element_center(element_id: &synapse_core::ElementId) -> Result<Point, ErrorData> {
    Err(action_error_to_mcp(&ActionError::BackendUnavailable {
        detail: format!(
            "act_click element target {element_id} requires Windows UI Automation bbox resolution"
        ),
    }))
}

fn element_is_coordinate_only(element_id: &synapse_core::ElementId) -> bool {
    crate::m1::is_browser_ocr_element_id(element_id)
}

#[derive(Default)]
struct ElementClickTraceReadback<'a> {
    fallback_screen_point: Option<Point>,
    state_before: Option<&'a str>,
    state_after: Option<&'a str>,
    selected_before: Option<bool>,
    selected_after: Option<bool>,
    legacy_default_action: Option<&'a str>,
}

fn trace_element_click_outcome(
    element: &ActClickElementTarget,
    click_index: u8,
    outcome: &'static str,
    readback: ElementClickTraceReadback<'_>,
) {
    tracing::info!(
        code = "M2_ACT_CLICK_ELEMENT_READBACK",
        kind = "act_click",
        element_id = %element.element_id,
        click_number = u32::from(click_index) + 1,
        outcome,
        state_before = readback.state_before,
        state_after = readback.state_after,
        selected_before = readback.selected_before,
        selected_after = readback.selected_after,
        legacy_default_action = readback.legacy_default_action,
        fallback_screen_x = readback.fallback_screen_point.map(|point| point.x),
        fallback_screen_y = readback.fallback_screen_point.map(|point| point.y),
        "readback=action_backend tool=act_click element_click_after"
    );
}

#[cfg(windows)]
#[derive(Clone, Debug)]
struct HwndMessageClickReadback {
    root_hwnd: i64,
    hwnd: i64,
    class_name: String,
    client_point: Point,
}

#[cfg(windows)]
#[derive(Clone, Debug)]
struct WindowCandidate {
    hwnd: HWND,
    rect: RECT,
    class_name: String,
}

#[cfg(windows)]
struct ChildEnumContext {
    point: Point,
    candidates: Vec<WindowCandidate>,
}

#[cfg(windows)]
fn windows_hwnd_message_click_readback(
    element_id: &synapse_core::ElementId,
    screen_point: Point,
    button: MouseButton,
) -> Result<HwndMessageClickReadback, ActionError> {
    let _ = mouse_button_messages(button)?;
    let root_hwnd = element_id
        .parts()
        .map_err(|error| ActionError::TargetInvalid {
            detail: format!("act_click element id {element_id} could not be parsed: {error}"),
        })?
        .hwnd;
    let root = hwnd_from_i64(root_hwnd)?;
    if !unsafe { IsWindow(Some(root)) }.as_bool() {
        return Err(ActionError::ElementNotResolved {
            detail: format!("act_click root hwnd 0x{root_hwnd:x} is not a live window"),
        });
    }

    let target = best_hwnd_for_screen_point(root, screen_point)?;
    let mut client = WinPoint {
        x: screen_point.x,
        y: screen_point.y,
    };
    if !unsafe { ScreenToClient(target.hwnd, &raw mut client) }.as_bool() {
        return Err(ActionError::BackendUnavailable {
            detail: format!(
                "ScreenToClient failed for act_click target hwnd 0x{:x} at screen point {screen_point:?}",
                hwnd_to_i64(target.hwnd)
            ),
        });
    }

    let client_point = Point {
        x: client.x,
        y: client.y,
    };
    let _ = mouse_lparam(client_point)?;
    Ok(HwndMessageClickReadback {
        root_hwnd,
        hwnd: hwnd_to_i64(target.hwnd),
        class_name: target.class_name,
        client_point,
    })
}

#[cfg(windows)]
fn best_hwnd_for_screen_point(root: HWND, point: Point) -> Result<WindowCandidate, ActionError> {
    let root_rect = window_rect(root)?;
    if !rect_contains_point(&root_rect, point) {
        return Err(ActionError::TargetInvalid {
            detail: format!(
                "act_click element center {point:?} is outside root hwnd 0x{:x} rect {:?}",
                hwnd_to_i64(root),
                rect_tuple(&root_rect)
            ),
        });
    }

    let mut context = ChildEnumContext {
        point,
        candidates: Vec::new(),
    };
    let context_ptr = (&raw mut context).cast::<c_void>();
    let _ = unsafe {
        EnumChildWindows(
            Some(root),
            Some(enum_child_containing_point),
            LPARAM(context_ptr as isize),
        )
    };

    context
        .candidates
        .into_iter()
        .min_by_key(|candidate| rect_area(&candidate.rect))
        .or_else(|| {
            Some(WindowCandidate {
                hwnd: root,
                rect: root_rect,
                class_name: window_class_name(root),
            })
        })
        .ok_or_else(|| ActionError::ElementNotResolved {
            detail: format!(
                "act_click could not find root or child hwnd containing point {point:?}"
            ),
        })
}

#[cfg(windows)]
unsafe extern "system" fn enum_child_containing_point(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let context = unsafe { &mut *(lparam.0 as *mut ChildEnumContext) };
    if unsafe { IsWindowVisible(hwnd) }.as_bool()
        && let Ok(rect) = window_rect(hwnd)
        && rect_contains_point(&rect, context.point)
        && rect_area(&rect) > 0
    {
        context.candidates.push(WindowCandidate {
            hwnd,
            rect,
            class_name: window_class_name(hwnd),
        });
    }
    BOOL(1)
}

#[cfg(windows)]
fn post_mouse_message(
    hwnd: i64,
    message: u32,
    wparam: usize,
    client_point: Point,
) -> Result<(), ActionError> {
    let hwnd = hwnd_from_i64(hwnd)?;
    let lparam = mouse_lparam(client_point)?;
    unsafe { PostMessageW(Some(hwnd), message, WPARAM(wparam), lparam) }.map_err(|error| {
        ActionError::BackendUnavailable {
            detail: format!(
                "PostMessageW act_click mouse message 0x{message:x} failed for hwnd 0x{:x} client_point={client_point:?}: {error}",
                hwnd_to_i64(hwnd)
            ),
        }
    })
}

#[cfg(windows)]
fn mouse_button_messages(button: MouseButton) -> Result<(u32, u32, usize), ActionError> {
    match button {
        MouseButton::Left => Ok((WM_LBUTTONDOWN, WM_LBUTTONUP, 0x0001)),
        MouseButton::Right => Ok((WM_RBUTTONDOWN, WM_RBUTTONUP, 0x0002)),
        MouseButton::Middle => Ok((WM_MBUTTONDOWN, WM_MBUTTONUP, 0x0010)),
        MouseButton::X1 | MouseButton::X2 => Err(ActionError::BackendUnavailable {
            detail: format!(
                "act_click HWND message fallback supports left/right/middle buttons only, got {button:?}"
            ),
        }),
    }
}

#[cfg(windows)]
fn mouse_lparam(client_point: Point) -> Result<LPARAM, ActionError> {
    let x = i16::try_from(client_point.x).map_err(|error| ActionError::TargetInvalid {
        detail: format!(
            "act_click client x {} cannot fit a WM_* mouse lParam i16: {error}",
            client_point.x
        ),
    })?;
    let y = i16::try_from(client_point.y).map_err(|error| ActionError::TargetInvalid {
        detail: format!(
            "act_click client y {} cannot fit a WM_* mouse lParam i16: {error}",
            client_point.y
        ),
    })?;
    let packed = (u32::from(u16::from_ne_bytes(y.to_ne_bytes())) << 16)
        | u32::from(u16::from_ne_bytes(x.to_ne_bytes()));
    Ok(LPARAM(isize::try_from(packed).unwrap_or(isize::MAX)))
}

#[cfg(windows)]
fn window_rect(hwnd: HWND) -> Result<RECT, ActionError> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &raw mut rect) }.map_err(|error| {
        ActionError::ElementNotResolved {
            detail: format!(
                "GetWindowRect failed for act_click hwnd 0x{:x}: {error}",
                hwnd_to_i64(hwnd)
            ),
        }
    })?;
    Ok(rect)
}

#[cfg(windows)]
fn rect_contains_point(rect: &RECT, point: Point) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

#[cfg(windows)]
fn rect_area(rect: &RECT) -> i64 {
    let width = i64::from(rect.right.saturating_sub(rect.left).max(0));
    let height = i64::from(rect.bottom.saturating_sub(rect.top).max(0));
    width.saturating_mul(height)
}

#[cfg(windows)]
fn rect_tuple(rect: &RECT) -> (i32, i32, i32, i32) {
    (rect.left, rect.top, rect.right, rect.bottom)
}

#[cfg(windows)]
fn window_class_name(hwnd: HWND) -> String {
    let mut buffer = vec![0_u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..usize::try_from(len).unwrap_or(0)])
}

#[cfg(windows)]
fn hwnd_from_i64(hwnd: i64) -> Result<HWND, ActionError> {
    if hwnd == 0 {
        return Err(ActionError::TargetInvalid {
            detail: "act_click element root hwnd is null".to_owned(),
        });
    }
    Ok(HWND(hwnd as isize as *mut c_void))
}

#[cfg(windows)]
fn hwnd_to_i64(hwnd: HWND) -> i64 {
    hwnd.0 as isize as i64
}

#[cfg(test)]
mod tests {
    use synapse_core::{AimCurve, AimNaturalParams, Backend, ButtonAction, MouseButton};

    use super::*;
    use crate::m2::click::schema::{
        ActClickTarget, ClickVelocityProfile, default_click_button, default_click_duration_ms,
        default_click_hold_ms, default_coordinate_fallback_on_unsupported,
        default_verify_timeout_ms,
    };

    #[test]
    fn direct_coordinate_element_click_uses_move_then_requested_presses() {
        let params = ActClickParams {
            target: ActClickTarget::Element(ActClickElementTarget {
                element_id: synapse_core::ElementId::parse("0x1000:0000002a00000001")
                    .expect("synthetic element id must be valid"),
            }),
            button: default_click_button(),
            clicks: 2,
            modifiers: Vec::new(),
            velocity_profile: ClickVelocityProfile::Natural,
            duration_ms: default_click_duration_ms(),
            hold_ms: default_click_hold_ms(),
            backend: Backend::Software,
            use_invoke_pattern: false,
            coordinate_fallback_on_unsupported: default_coordinate_fallback_on_unsupported(),
            verify_delta: false,
            verify_timeout_ms: default_verify_timeout_ms(),
            deprecated_curve_alias_used: false,
        };
        let screen_point = Point { x: 320, y: 240 };
        let before = "screen_point=(320,240), clicks=2";

        let after = coordinate_click_actions(&params, screen_point);

        assert_eq!(after.len(), 3);
        assert!(matches!(
            after[0],
            Action::MouseMove {
                to: MouseTarget::Screen {
                    point: Point { x: 320, y: 240 }
                },
                curve: AimCurve::Natural {
                    params: AimNaturalParams::FAST
                },
                duration_ms: 50,
                backend: Backend::Software,
            }
        ));
        for action in &after[1..] {
            assert!(matches!(
                action,
                Action::MouseButton {
                    button: MouseButton::Left,
                    action: ButtonAction::Press,
                    hold_ms: 120,
                    backend: Backend::Software,
                }
            ));
        }
        println!("readback=act_click_element_coordinate_direct before={before} after={after:?}");
    }

    #[test]
    fn coordinate_fallback_allows_enabled_focusable_edit_with_value_pattern() {
        let metadata = synapse_a11y::ElementMetadataReadback {
            name: "File name:".to_owned(),
            role: "edit".to_owned(),
            automation_id: Some("1148".to_owned()),
            bbox: synapse_core::Rect {
                x: 10,
                y: 20,
                w: 200,
                h: 24,
            },
            enabled: true,
            keyboard_focusable: true,
            patterns: vec![synapse_core::UiaPattern::Value],
            value: Some("before.txt".to_owned()),
        };

        let allowed = coordinate_fallback_allowed_for_metadata(&metadata);

        println!(
            "readback=act_click_coordinate_fallback edge=enabled_edit metadata={metadata:?} allowed={allowed}"
        );
        assert!(allowed);
    }

    #[test]
    fn coordinate_fallback_denies_non_text_button() {
        let metadata = synapse_a11y::ElementMetadataReadback {
            name: "OK".to_owned(),
            role: "button".to_owned(),
            automation_id: Some("1".to_owned()),
            bbox: synapse_core::Rect {
                x: 10,
                y: 20,
                w: 80,
                h: 24,
            },
            enabled: true,
            keyboard_focusable: true,
            patterns: vec![synapse_core::UiaPattern::Invoke],
            value: None,
        };

        let allowed = coordinate_fallback_allowed_for_metadata(&metadata);

        println!(
            "readback=act_click_coordinate_fallback edge=non_text_button metadata={metadata:?} allowed={allowed}"
        );
        assert!(!allowed);
    }

    #[test]
    fn coordinate_fallback_denies_non_focusable_value_element() {
        let metadata = synapse_a11y::ElementMetadataReadback {
            name: "Readout".to_owned(),
            role: "text".to_owned(),
            automation_id: None,
            bbox: synapse_core::Rect {
                x: 10,
                y: 20,
                w: 120,
                h: 24,
            },
            enabled: true,
            keyboard_focusable: false,
            patterns: vec![synapse_core::UiaPattern::Value],
            value: Some("42".to_owned()),
        };

        let allowed = coordinate_fallback_allowed_for_metadata(&metadata);

        println!(
            "readback=act_click_coordinate_fallback edge=non_focusable_value metadata={metadata:?} allowed={allowed}"
        );
        assert!(!allowed);
    }

    #[cfg(windows)]
    #[test]
    fn stale_bbox_resolution_maps_to_transient_expired() {
        let element_id = synapse_core::ElementId::parse("0x1000:0000002a00000001")
            .expect("synthetic element id must be valid");
        let before = synapse_a11y::A11yError::ElementStale {
            detail: format!("element id {element_id} was not found under hwnd 0x1000"),
        };

        let after = element_resolution_error(&element_id, before);

        assert_eq!(
            after.code(),
            synapse_core::error_codes::TRANSIENT_ELEMENT_EXPIRED
        );
        assert!(after.detail().contains(element_id.as_str()));
        println!(
            "readback=act_click_element_bbox edge=stale_resolution before=ElementStale after_code={} after_detail={:?}",
            after.code(),
            after.detail()
        );
    }
}
