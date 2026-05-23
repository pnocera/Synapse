use std::{sync::Arc, time::Instant};

use rmcp::ErrorData;
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use synapse_action::{
    ActionBackend, ActionError, ActionHandle, EmitState, RecordedInput, RecordingBackend,
};
use synapse_core::{Action, Backend, Point};

use crate::m1::mcp_error;

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActScrollParams {
    #[serde(default)]
    #[schemars(default)]
    pub dy: i32,
    #[serde(default)]
    #[schemars(default)]
    pub dx: i32,
    pub at: Option<ActScrollPoint>,
    #[serde(default)]
    #[schemars(default)]
    pub smooth: bool,
}

#[derive(Copy, Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActScrollPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActScrollResponse {
    pub ok: bool,
    pub dy: i32,
    pub dx: i32,
    pub smooth: bool,
    pub scrolled: bool,
    pub backend_used: String,
    pub elapsed_ms: u32,
}

pub async fn act_scroll_with_handle(
    handle: ActionHandle,
    recording: Option<Arc<RecordingBackend>>,
    params: ActScrollParams,
) -> Result<ActScrollResponse, ErrorData> {
    validate_scroll_params(&params)?;
    let started = Instant::now();
    if params.dy == 0 && params.dx == 0 {
        if let Some(recording) = recording {
            execute_recording_noop(&recording);
        }
        return Ok(response(&params, false, "none", started));
    }

    let action = Action::MouseScroll {
        dy: params.dy,
        dx: params.dx,
        at: params.at.map(Into::into),
        backend: Backend::Auto,
    };

    if let Some(recording) = recording {
        execute_recording(&recording, &action)?;
    } else {
        handle
            .execute(action)
            .await
            .map_err(|error| action_error_to_mcp(&error))?;
    }

    Ok(response(&params, true, "software", started))
}

impl From<ActScrollPoint> for Point {
    fn from(value: ActScrollPoint) -> Self {
        Self {
            x: value.x,
            y: value.y,
        }
    }
}

fn validate_scroll_params(params: &ActScrollParams) -> Result<(), ErrorData> {
    if params.smooth && (params.dy != 0 || params.dx != 0) {
        return Err(action_error_to_mcp(&ActionError::BackendUnavailable {
            detail: "act_scroll smooth=true requires the dedicated smooth wheel implementation"
                .to_owned(),
        }));
    }
    Ok(())
}

fn response(
    params: &ActScrollParams,
    scrolled: bool,
    backend_used: &'static str,
    started: Instant,
) -> ActScrollResponse {
    ActScrollResponse {
        ok: true,
        dy: params.dy,
        dx: params.dx,
        smooth: params.smooth,
        scrolled,
        backend_used: backend_used.to_owned(),
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    }
}

fn execute_recording(recording: &RecordingBackend, action: &Action) -> Result<(), ErrorData> {
    let before_events = recording.events();
    let before_event_count = before_events.len();
    let mut emit_state = EmitState::new();
    recording
        .execute(action, &mut emit_state)
        .map_err(|error| action_error_to_mcp(&error))?;
    let after_events = recording.events();
    let new_events = &after_events[before_event_count..];
    log_recording_readback(before_event_count, &after_events, new_events);
    Ok(())
}

fn execute_recording_noop(recording: &RecordingBackend) {
    let before_events = recording.events();
    let before_event_count = before_events.len();
    let after_events = recording.events();
    let new_events = &after_events[before_event_count..];
    log_recording_readback(before_event_count, &after_events, new_events);
}

fn log_recording_readback(
    before_event_count: usize,
    after_events: &[RecordedInput],
    new_events: &[RecordedInput],
) {
    let event_sequence = event_sequence(new_events);
    tracing::info!(
        code = "M2_ACT_SCROLL_RECORDING_READBACK",
        kind = "act_scroll",
        before_event_count,
        after_event_count = after_events.len(),
        new_event_count = new_events.len(),
        event_sequence,
        ?new_events,
        "source_of_truth=recording_backend tool=act_scroll after_events_readback"
    );
}

fn event_sequence(events: &[RecordedInput]) -> String {
    events.iter().map(event_label).collect::<Vec<_>>().join(">")
}

fn event_label(event: &RecordedInput) -> String {
    match event {
        RecordedInput::MouseScroll { dy, dx, at } => {
            format!("mouse_scroll:dy={dy}:dx={dx}:at={}", at_label(*at))
        }
        other => format!("{other:?}"),
    }
}

fn at_label(at: Option<Point>) -> String {
    at.map_or_else(
        || "none".to_owned(),
        |point| format!("screen({},{})", point.x, point.y),
    )
}

fn action_error_to_mcp(error: &ActionError) -> ErrorData {
    mcp_error(error.code(), error.to_string())
}
