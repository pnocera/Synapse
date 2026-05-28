use std::time::{Duration, Instant};

use rmcp::{ErrorData, model::ErrorCode, schemars::JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use synapse_action::{ActionBackend, EmitState};
use synapse_core::{Action, Backend, FocusedElement, Key, KeyCode, error_codes};
use synapse_everquest::{EverQuestLogKind, EverQuestLogTailBatch, tail_log};
use tokio::time::sleep;

use super::{
    Json, Parameters, SynapseService, everquest_log::EVERQUEST_PROFILE_ID, tool, tool_router,
};
use crate::m1::{current_input, mcp_error};

const TOOL: &str = "everquest_loc_probe";
const LOC_COMMAND: &str = "/loc";
const LOC_KEY_HOLD_MS: u32 = 33;
const LOC_INTER_KEY_DELAY: Duration = Duration::from_millis(20);
const MAX_LOC_LOG_BYTES: usize = 64 * 1024;
const MAX_LOC_LOG_EVENTS: usize = 128;
const LOC_POLL_INTERVAL: Duration = Duration::from_millis(100);
const LOC_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestLocProbeParams {}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestLocProbeResponse {
    pub ok: bool,
    pub command: String,
    pub coordinate_order: String,
    pub log_path: String,
    pub start_offset: u64,
    pub next_offset: u64,
    pub file_len_bytes: u64,
    pub bytes_read: usize,
    pub event_count: usize,
    pub you_say_count: usize,
    pub location: EverQuestLocProbeLocation,
    pub elapsed_ms: u32,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestLocProbeLocation {
    pub display_y: f64,
    pub display_x: f64,
    pub display_z: f64,
    pub log_timestamp: String,
    pub summary: String,
}

#[tool_router(router = everquest_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Send the literal EverQuest /loc command to the foreground everquest.live window and verify the appended EQ log coordinate line"
    )]
    pub async fn everquest_loc_probe(
        &self,
        _params: Parameters<EverQuestLocProbeParams>,
    ) -> Result<Json<EverQuestLocProbeResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = TOOL,
            "tool.invocation kind=everquest_loc_probe"
        );

        let request_details = json!({
            "command": LOC_COMMAND,
            "literal_only": true,
            "free_text_allowed": false,
            "coordinate_order": "everquest_display_y_x_z",
        });

        if let Err(error) = self.ensure_supported_use_allows_action(TOOL) {
            self.audit_action_denied_with_details(TOOL, &error, &request_details);
            return Err(error);
        }
        if let Err(error) = self.ensure_active_everquest_profile() {
            self.audit_action_denied_with_details(TOOL, &error, &request_details);
            return Err(error);
        }
        if let Err(error) = self.ensure_loc_probe_chat_guard() {
            self.audit_action_denied_with_details(TOOL, &error, &request_details);
            return Err(error);
        }

        let active = match self.resolve_active_everquest_log() {
            Ok(active) => active,
            Err(detail) => {
                let error = loc_probe_error("active_log_unavailable", detail, &json!({}));
                self.audit_action_denied_with_details(TOOL, &error, &request_details);
                return Err(error);
            }
        };
        let start_offset = std::fs::metadata(&active.log.path)
            .map_err(|error| {
                loc_probe_error(
                    "log_metadata_unreadable",
                    format!("read active EverQuest log metadata: {error}"),
                    &json!({ "path": active.log.path.display().to_string() }),
                )
            })?
            .len();

        self.audit_action_started_with_details(
            TOOL,
            &json!({
                "request": request_details,
                "log_path": active.log.path.display().to_string(),
                "start_offset": start_offset,
            }),
        )?;

        let started = Instant::now();
        let result = async {
            self.execute_literal_loc_command().await?;
            self.read_loc_probe_result(
                &active.log.path,
                start_offset,
                u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
            )
            .await
        }
        .await;

        self.audit_action_result(TOOL, &result)?;
        result.map(Json)
    }
}

impl SynapseService {
    fn ensure_active_everquest_profile(&self) -> Result<(), ErrorData> {
        let runtime = self.profile_runtime()?;
        let active_profile_id = runtime
            .active_profile_id()
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        if active_profile_id.as_deref() == Some(EVERQUEST_PROFILE_ID) {
            return Ok(());
        }
        Err(loc_probe_error(
            "active_profile_mismatch",
            format!("{TOOL} requires active profile {EVERQUEST_PROFILE_ID}"),
            &json!({
                "active_profile_id": active_profile_id,
                "required_profile_id": EVERQUEST_PROFILE_ID,
            }),
        ))
    }

    fn ensure_loc_probe_chat_guard(&self) -> Result<(), ErrorData> {
        let input = {
            let state = self.m1_state()?;
            current_input(&state, 1)?
        };
        if let Some(reason) = focused_text_entry_pollution_reason(input.focused.as_ref()) {
            return Err(loc_probe_error(
                "focused_text_entry_not_empty",
                format!("{TOOL} refused to append /loc into an existing focused text entry"),
                &json!({ "focused_text_entry_reason": reason }),
            ));
        }
        Ok(())
    }

    async fn execute_literal_loc_command(&self) -> Result<(), ErrorData> {
        let (handle, recording, _connection_closed_cancel) = self.m2_action_context()?;
        let actions = literal_loc_actions();
        if let Some(recording) = recording {
            let mut emit_state = EmitState::new();
            for action in &actions {
                recording
                    .execute(action, &mut emit_state)
                    .map_err(|error| mcp_error(error.code(), error.to_string()))?;
            }
            return Ok(());
        }
        for action in actions {
            handle
                .execute(action)
                .await
                .map_err(|error| mcp_error(error.code(), error.to_string()))?;
            sleep(LOC_INTER_KEY_DELAY).await;
        }
        Ok(())
    }

    async fn read_loc_probe_result(
        &self,
        log_path: &std::path::Path,
        start_offset: u64,
        initial_elapsed_ms: u32,
    ) -> Result<EverQuestLocProbeResponse, ErrorData> {
        let started = Instant::now();
        let last_batch = loop {
            let batch = tail_log(
                log_path,
                start_offset,
                MAX_LOC_LOG_BYTES,
                MAX_LOC_LOG_EVENTS,
            )
            .map_err(|error| {
                loc_probe_error(
                    "log_tail_failed",
                    format!("tail active EverQuest log after /loc: {error}"),
                    &json!({
                        "path": log_path.display().to_string(),
                        "start_offset": start_offset,
                    }),
                )
            })?;
            let you_say_count = you_say_count(&batch);
            if you_say_count > 0 {
                return Err(loc_probe_error(
                    "chat_pollution_detected",
                    format!("{TOOL} detected player say output after /loc dispatch"),
                    &log_batch_detail(&batch, you_say_count),
                ));
            }
            if let Some(response) = loc_probe_response_from_batch(
                &batch,
                you_say_count,
                elapsed_ms_since(initial_elapsed_ms, started),
            ) {
                return Ok(response);
            }
            if started.elapsed() >= LOC_TIMEOUT {
                break batch;
            }
            sleep(LOC_POLL_INTERVAL).await;
        };

        Err(loc_probe_error(
            "location_log_line_absent",
            format!("{TOOL} did not observe a /loc coordinate line before timeout"),
            &log_batch_detail(&last_batch, 0),
        ))
    }
}

fn focused_text_entry_pollution_reason(focused: Option<&FocusedElement>) -> Option<String> {
    let focused = focused?;
    let role = focused.role.to_ascii_lowercase();
    let name = focused.name.to_ascii_lowercase();
    let is_text_entry = role.contains("edit")
        || role.contains("text")
        || role.contains("document")
        || name.contains("chat")
        || focused.patterns.iter().any(|pattern| {
            matches!(
                pattern,
                synapse_core::UiaPattern::Text | synapse_core::UiaPattern::Value
            )
        });
    if !is_text_entry {
        return None;
    }
    let value_len = focused.value.as_deref().map_or("", str::trim).len();
    let selected_len = focused.selected_text.as_deref().map_or("", str::trim).len();
    (value_len > 0 || selected_len > 0).then(|| {
        format!(
            "focused role={:?} name={:?} value_len={} selected_len={}",
            focused.role, focused.name, value_len, selected_len
        )
    })
}

fn literal_loc_actions() -> Vec<Action> {
    [
        loc_key(KeyCode::Symbol { value: '/' }),
        loc_key(KeyCode::Symbol { value: 'l' }),
        loc_key(KeyCode::Symbol { value: 'o' }),
        loc_key(KeyCode::Symbol { value: 'c' }),
        loc_key(KeyCode::Named {
            value: "enter".to_owned(),
        }),
    ]
    .into_iter()
    .map(|key| Action::KeyPress {
        key,
        hold_ms: LOC_KEY_HOLD_MS,
        backend: Backend::Auto,
    })
    .collect()
}

const fn loc_key(code: KeyCode) -> Key {
    Key {
        code,
        use_scancode: false,
    }
}

fn loc_probe_response_from_batch(
    batch: &EverQuestLogTailBatch,
    you_say_count: usize,
    elapsed_ms: u32,
) -> Option<EverQuestLocProbeResponse> {
    let event = batch
        .events
        .iter()
        .find(|event| event.kind == EverQuestLogKind::Location)?;
    let location = event.location.as_ref()?;
    Some(EverQuestLocProbeResponse {
        ok: true,
        command: LOC_COMMAND.to_owned(),
        coordinate_order: "everquest_display_y_x_z".to_owned(),
        log_path: batch.path.display().to_string(),
        start_offset: batch.start_offset,
        next_offset: batch.next_offset,
        file_len_bytes: batch.file_len_bytes,
        bytes_read: batch.bytes_read,
        event_count: batch.events.len(),
        you_say_count,
        location: EverQuestLocProbeLocation {
            display_y: location.display_y,
            display_x: location.display_x,
            display_z: location.display_z,
            log_timestamp: event.timestamp.format("%Y-%m-%dT%H:%M:%S").to_string(),
            summary: event.summary.clone(),
        },
        elapsed_ms,
    })
}

fn elapsed_ms_since(initial_elapsed_ms: u32, started: Instant) -> u32 {
    let poll_elapsed_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
    initial_elapsed_ms.saturating_add(poll_elapsed_ms)
}

fn you_say_count(batch: &EverQuestLogTailBatch) -> usize {
    batch
        .events
        .iter()
        .filter(|event| {
            event.kind == EverQuestLogKind::Say
                && event
                    .actor
                    .as_deref()
                    .is_some_and(|actor| actor.eq_ignore_ascii_case("you"))
        })
        .count()
}

fn log_batch_detail(batch: &EverQuestLogTailBatch, you_say_count: usize) -> Value {
    json!({
        "path": batch.path.display().to_string(),
        "start_offset": batch.start_offset,
        "next_offset": batch.next_offset,
        "file_len_bytes": batch.file_len_bytes,
        "bytes_read": batch.bytes_read,
        "event_count": batch.events.len(),
        "you_say_count": you_say_count,
        "truncated_by_bytes": batch.truncated_by_bytes,
        "truncated_by_events": batch.truncated_by_events,
    })
}

fn loc_probe_error(reason: &'static str, message: impl Into<String>, detail: &Value) -> ErrorData {
    ErrorData::new(
        ErrorCode(-32099),
        message.into(),
        Some(json!({
            "code": error_codes::ACTION_TARGET_INVALID,
            "tool": TOOL,
            "reason": reason,
            "detail": detail,
        })),
    )
}

#[cfg(test)]
mod tests {
    use synapse_core::{Rect, UiaPattern, element_id};

    use super::*;

    #[test]
    fn loc_batch_response_uses_structured_location_and_counts_chat() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("eqlog_Thenumberone_frostreaver.txt");
        std::fs::write(
            &path,
            "[Thu May 28 11:00:00 2026] Your Location is -1.25, 2.50, 3.75\r\n",
        )?;
        let batch = tail_log(&path, 0, MAX_LOC_LOG_BYTES, MAX_LOC_LOG_EVENTS)?;

        let response = loc_probe_response_from_batch(&batch, you_say_count(&batch), 7)
            .unwrap_or_else(|| panic!("expected loc response"));

        assert_eq!(response.location.display_y, -1.25);
        assert_eq!(response.location.display_x, 2.5);
        assert_eq!(response.location.display_z, 3.75);
        assert_eq!(response.you_say_count, 0);
        assert_eq!(response.elapsed_ms, 7);
        Ok(())
    }

    #[test]
    fn chat_guard_denies_nonempty_focused_text_entry() {
        let focused = FocusedElement {
            element_id: element_id(7, "cafe"),
            name: "Chat Input".to_owned(),
            role: "Edit".to_owned(),
            automation_id: None,
            bbox: Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 20,
            },
            enabled: true,
            patterns: vec![UiaPattern::Text, UiaPattern::Value],
            value: Some("partial text".to_owned()),
            selected_text: None,
        };

        let reason = focused_text_entry_pollution_reason(Some(&focused))
            .unwrap_or_else(|| panic!("expected focused text pollution reason"));

        assert!(reason.contains("value_len=12"));
    }

    #[test]
    fn chat_guard_allows_empty_focused_text_entry_for_literal_command() {
        let focused = FocusedElement {
            element_id: element_id(7, "cafe"),
            name: "Chat Input".to_owned(),
            role: "Edit".to_owned(),
            automation_id: None,
            bbox: Rect {
                x: 0,
                y: 0,
                w: 100,
                h: 20,
            },
            enabled: true,
            patterns: vec![UiaPattern::Text, UiaPattern::Value],
            value: Some("   ".to_owned()),
            selected_text: None,
        };

        assert!(focused_text_entry_pollution_reason(Some(&focused)).is_none());
    }

    #[test]
    fn literal_loc_actions_are_fixed_keypress_sequence() {
        let actions = literal_loc_actions();
        let keys = actions
            .iter()
            .map(|action| match action {
                Action::KeyPress { key, hold_ms, .. } => {
                    assert_eq!(*hold_ms, LOC_KEY_HOLD_MS);
                    match &key.code {
                        KeyCode::Symbol { value } => value.to_string(),
                        KeyCode::Named { value } => value.clone(),
                        KeyCode::HidCode { value } => value.to_string(),
                    }
                }
                other => panic!("unexpected /loc action: {other:?}"),
            })
            .collect::<Vec<_>>();

        assert_eq!(keys, ["/", "l", "o", "c", "enter"]);
    }
}
