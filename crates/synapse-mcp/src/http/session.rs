use std::{sync::Arc, time::Duration};

use anyhow::{Context, bail};
use axum::{
    body::{Body, to_bytes},
    extract::State,
    http::{Method, Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use rmcp::transport::streamable_http_server::session::local::{LocalSessionManager, SessionConfig};
use synapse_action::ActionHandle;

const SESSION_IDLE_TIMEOUT_ENV: &str = "SYNAPSE_HTTP_SESSION_IDLE_TIMEOUT_SECS";
const DEFAULT_SESSION_IDLE_TIMEOUT_SECS: u64 = 5 * 60;
const MAX_MCP_REQUEST_BYTES: usize = 1024 * 1024;
const SESSION_ID_HEADER: &str = "Mcp-Session-Id";

tokio::task_local! {
    static CURRENT_MCP_SESSION_ID: Option<String>;
}

#[derive(Clone)]
pub(super) struct SessionRequestState {
    session_registry: crate::server::session_registry::SharedSessionRegistry,
}

#[derive(Clone)]
pub(super) struct SessionCleanupState {
    action_handle: ActionHandle,
    session_manager: Arc<LocalSessionManager>,
    session_targets: crate::server::SharedSessionTargets,
    cdp_target_owners: crate::server::SharedCdpTargetOwners,
    session_registry: crate::server::session_registry::SharedSessionRegistry,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum SessionFailure {
    Missing,
    UnknownOrExpired,
}

impl SessionCleanupState {
    pub(super) fn request_state(
        session_registry: crate::server::session_registry::SharedSessionRegistry,
    ) -> SessionRequestState {
        SessionRequestState { session_registry }
    }

    pub(super) fn new(
        action_handle: ActionHandle,
        session_manager: Arc<LocalSessionManager>,
        session_targets: crate::server::SharedSessionTargets,
        cdp_target_owners: crate::server::SharedCdpTargetOwners,
        session_registry: crate::server::session_registry::SharedSessionRegistry,
    ) -> Self {
        Self {
            action_handle,
            session_manager,
            session_targets,
            cdp_target_owners,
            session_registry,
        }
    }
}

pub(crate) fn current_mcp_session_id() -> Option<String> {
    CURRENT_MCP_SESSION_ID.try_with(Clone::clone).ok().flatten()
}

pub(super) fn load_session_config() -> anyhow::Result<SessionConfig> {
    let mut config = SessionConfig::default();
    let idle_timeout_secs = session_idle_timeout_secs()?;
    config.keep_alive = Some(Duration::from_secs(idle_timeout_secs));
    tracing::info!(
        code = "MCP_HTTP_SESSION_CONFIGURED",
        idle_timeout_s = idle_timeout_secs,
        "HTTP MCP session lifecycle configured"
    );
    Ok(config)
}

pub(super) async fn require_mcp_session(
    State(state): State<SessionRequestState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if !is_mcp_endpoint(request.uri().path()) {
        return next.run(request).await;
    }
    let session_id = session_id_from_header(&request);
    let request = match enforce_session_header(request).await {
        Ok(request) => request,
        Err(response) => return response,
    };
    let request = match session_id.as_deref() {
        Some(session_id) => {
            match record_session_request(&state.session_registry, session_id, request).await {
                Ok(request) => request,
                Err(response) => return response,
            }
        }
        None => request,
    };
    CURRENT_MCP_SESSION_ID
        .scope(session_id, async move {
            let response = next.run(request).await;
            if response.status() == StatusCode::NOT_FOUND {
                return session_invalid(SessionFailure::UnknownOrExpired);
            }
            response
        })
        .await
}

pub(super) async fn release_held_inputs_on_delete(
    State(state): State<SessionCleanupState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let cleanup_session_id = (request.method() == Method::DELETE
        && is_mcp_endpoint(request.uri().path()))
    .then(|| session_id_from_header(&request))
    .flatten();
    if let Some(session_id) = cleanup_session_id.as_deref()
        && !session_is_active(&state.session_manager, session_id).await
    {
        tracing::warn!(
            code = synapse_core::error_codes::HTTP_SESSION_INVALID,
            session_id,
            reason = ?SessionFailure::UnknownOrExpired,
            "HTTP MCP session delete rejected before held-input cleanup"
        );
        return session_invalid(SessionFailure::UnknownOrExpired);
    }
    let response = next.run(request).await;
    let Some(session_id) = cleanup_session_id else {
        return response;
    };
    if !response.status().is_success() {
        return response;
    }

    let before = match state.action_handle.session_inputs_snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::error!(
                code = error.code(),
                session_id,
                detail = %error.detail(),
                "HTTP MCP session cleanup could not read held-input ownership before release"
            );
            return cleanup_failed(error);
        }
    };
    let result = state
        .action_handle
        .release_session_inputs_and_lease(&session_id)
        .await;
    let after = state.action_handle.session_inputs_snapshot();
    // Drop the session's active perception target so the registry does not leak
    // entries for disconnected agents (epic #720).
    let target_cleared = state
        .session_targets
        .lock()
        .is_ok_and(|mut targets| targets.remove(&session_id).is_some());
    let registry_closed = match state.session_registry.lock() {
        Ok(mut registry) => {
            registry.record_closed(
                &session_id,
                crate::server::session_registry::unix_time_ms_now(),
            );
            true
        }
        Err(_error) => {
            tracing::error!(
                code = synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                session_id,
                "HTTP MCP session cleanup could not lock cross-session registry"
            );
            false
        }
    };
    let cdp_cleanup =
        cleanup_session_cdp_targets(&state.cdp_target_owners, &session_id, "http_delete").await;
    match result {
        Ok(summary) => {
            tracing::info!(
                code = "MCP_HTTP_SESSION_INPUT_CLEANUP",
                session_id,
                released_keys = summary.input_summary.released_keys,
                released_buttons = summary.input_summary.released_buttons,
                neutralized_pads = summary.input_summary.neutralized_pads,
                retained_shared_inputs = summary.input_summary.retained_shared_inputs,
                input_lease_released = summary.lease_released,
                expired_lease_cleanup_completed = summary.expired_lease_cleanup_completed,
                session_target_cleared = target_cleared,
                session_registry_closed = registry_closed,
                before = ?before,
                after = ?after,
                cdp_cleanup_reason = cdp_cleanup.reason,
                cdp_owned_before = cdp_cleanup.owned_before,
                cdp_closed = cdp_cleanup.closed,
                cdp_failed = cdp_cleanup.failed,
                cdp_target_ids = ?cdp_cleanup.target_ids,
                "readback=session_input_ownership edge=http_delete after_cleanup"
            );
            response
        }
        Err(error) => {
            tracing::error!(
                code = error.code(),
                session_id,
                detail = %error.detail(),
                before = ?before,
                after = ?after,
                "HTTP MCP session cleanup failed while releasing owned inputs"
            );
            cleanup_failed(error)
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct SessionCdpTargetCleanupReadback {
    pub reason: &'static str,
    pub owned_before: usize,
    pub closed: usize,
    pub failed: usize,
    pub target_ids: Vec<String>,
}

pub(super) async fn cleanup_session_cdp_targets(
    cdp_target_owners: &crate::server::SharedCdpTargetOwners,
    session_id: &str,
    reason: &'static str,
) -> SessionCdpTargetCleanupReadback {
    let owned = match remove_session_cdp_target_owners(cdp_target_owners, session_id) {
        Ok(owned) => owned,
        Err(detail) => {
            tracing::error!(
                code = synapse_core::error_codes::TOOL_INTERNAL_ERROR,
                session_id,
                reason,
                detail = %detail,
                "HTTP MCP session cleanup could not lock CDP target ownership registry"
            );
            return SessionCdpTargetCleanupReadback {
                reason,
                owned_before: 0,
                closed: 0,
                failed: 1,
                target_ids: Vec::new(),
            };
        }
    };
    let target_ids = owned
        .iter()
        .map(|(target_id, _owner)| target_id.clone())
        .collect::<Vec<_>>();
    let owned_before = owned.len();
    let mut closed = 0_usize;
    let mut failed = 0_usize;
    for (target_id, owner) in owned {
        match close_cdp_target_for_cleanup(&target_id, &owner).await {
            Ok(()) => {
                closed = closed.saturating_add(1);
                tracing::info!(
                    code = "MCP_HTTP_SESSION_CDP_TARGET_CLEANUP",
                    session_id,
                    reason,
                    hwnd = owner.window_hwnd,
                    endpoint = %owner.endpoint,
                    cdp_target_id = %target_id,
                    "readback=Target.closeTarget edge=session_cleanup after=closed"
                );
            }
            Err(detail) => {
                failed = failed.saturating_add(1);
                tracing::error!(
                    code = synapse_core::error_codes::A11Y_CDP_AXTREE_FAILED,
                    session_id,
                    reason,
                    hwnd = owner.window_hwnd,
                    endpoint = %owner.endpoint,
                    cdp_target_id = %target_id,
                    detail = %detail,
                    "HTTP MCP session cleanup removed CDP owner but failed to close target"
                );
            }
        }
    }
    SessionCdpTargetCleanupReadback {
        reason,
        owned_before,
        closed,
        failed,
        target_ids,
    }
}

fn remove_session_cdp_target_owners(
    cdp_target_owners: &crate::server::SharedCdpTargetOwners,
    session_id: &str,
) -> Result<Vec<(String, crate::server::CdpTargetOwner)>, String> {
    let mut guard = cdp_target_owners
        .lock()
        .map_err(|_error| "CDP target ownership registry lock poisoned".to_owned())?;
    let owned_ids = guard
        .iter()
        .filter_map(|(target_id, owner)| {
            (owner.session_id == session_id).then(|| target_id.clone())
        })
        .collect::<Vec<_>>();
    let owned = owned_ids
        .into_iter()
        .filter_map(|target_id| guard.remove(&target_id).map(|owner| (target_id, owner)))
        .collect();
    drop(guard);
    Ok(owned)
}

#[cfg(windows)]
async fn close_cdp_target_for_cleanup(
    target_id: &str,
    owner: &crate::server::CdpTargetOwner,
) -> Result<(), String> {
    synapse_a11y::cdp_close_target(&owner.endpoint, target_id)
        .await
        .map(|_closed| ())
        .map_err(|error| error.to_string())
}

#[cfg(not(windows))]
async fn close_cdp_target_for_cleanup(
    target_id: &str,
    owner: &crate::server::CdpTargetOwner,
) -> Result<(), String> {
    Err(format!(
        "CDP target cleanup is only available on Windows; target_id={target_id:?} endpoint={:?}",
        owner.endpoint
    ))
}

async fn session_is_active(session_manager: &LocalSessionManager, session_id: &str) -> bool {
    session_manager
        .sessions
        .read()
        .await
        .contains_key(session_id)
}

fn session_idle_timeout_secs() -> anyhow::Result<u64> {
    match std::env::var(SESSION_IDLE_TIMEOUT_ENV) {
        Ok(raw) => parse_idle_timeout(&raw)
            .with_context(|| format!("parse {SESSION_IDLE_TIMEOUT_ENV}={raw:?}")),
        Err(std::env::VarError::NotPresent) => Ok(DEFAULT_SESSION_IDLE_TIMEOUT_SECS),
        Err(error) => Err(error).with_context(|| format!("read {SESSION_IDLE_TIMEOUT_ENV}")),
    }
}

fn parse_idle_timeout(raw: &str) -> anyhow::Result<u64> {
    let value = raw.trim();
    let seconds = value
        .parse::<u64>()
        .with_context(|| format!("invalid integer {value:?}"))?;
    if seconds == 0 {
        bail!("idle timeout must be greater than zero seconds");
    }
    Ok(seconds)
}

async fn enforce_session_header(request: Request<Body>) -> Result<Request<Body>, Response> {
    if has_session_header(&request) {
        return Ok(request);
    }
    if request.method() == Method::POST {
        allow_initialize_without_session(request).await
    } else if request.method() == Method::GET || request.method() == Method::DELETE {
        Err(session_invalid(SessionFailure::Missing))
    } else {
        Ok(request)
    }
}

fn has_session_header(request: &Request<Body>) -> bool {
    session_id_from_header(request).is_some()
}

fn session_id_from_header(request: &Request<Body>) -> Option<String> {
    request
        .headers()
        .get(SESSION_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

async fn allow_initialize_without_session(
    request: Request<Body>,
) -> Result<Request<Body>, Response> {
    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, MAX_MCP_REQUEST_BYTES)
        .await
        .map_err(|_| payload_too_large())?;
    let parsed = serde_json::from_slice::<serde_json::Value>(&bytes);
    let is_initialize = parsed.as_ref().is_ok_and(jsonrpc_method_is_initialize);
    let request = Request::from_parts(parts, Body::from(bytes));
    if parsed.is_err() || is_initialize {
        Ok(request)
    } else {
        Err(session_invalid(SessionFailure::Missing))
    }
}

async fn record_session_request(
    session_registry: &crate::server::session_registry::SharedSessionRegistry,
    session_id: &str,
    request: Request<Body>,
) -> Result<Request<Body>, Response> {
    if request.method() != Method::POST {
        record_session_heartbeat(session_registry, session_id, None)?;
        return Ok(request);
    }

    let (parts, body) = request.into_parts();
    let bytes = to_bytes(body, MAX_MCP_REQUEST_BYTES)
        .await
        .map_err(|_| payload_too_large())?;
    let action = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|value| jsonrpc_action_label(&value));
    record_session_heartbeat(session_registry, session_id, action)?;
    Ok(Request::from_parts(parts, Body::from(bytes)))
}

fn record_session_heartbeat(
    session_registry: &crate::server::session_registry::SharedSessionRegistry,
    session_id: &str,
    action: Option<String>,
) -> Result<(), Response> {
    let mut registry = session_registry.lock().map_err(|_error| {
        tracing::error!(
            code = synapse_core::error_codes::TOOL_INTERNAL_ERROR,
            session_id,
            "HTTP MCP session request could not lock cross-session registry"
        );
        session_registry_failed()
    })?;
    registry.record_seen(
        session_id,
        action,
        crate::server::session_registry::unix_time_ms_now(),
    );
    Ok(())
}

fn jsonrpc_action_label(value: &serde_json::Value) -> Option<String> {
    if value.is_array() {
        return Some("jsonrpc_batch".to_owned());
    }
    let method = value.get("method")?.as_str()?;
    if method == "tools/call"
        && let Some(name) = value
            .get("params")
            .and_then(|params| params.get("name"))
            .and_then(serde_json::Value::as_str)
    {
        return Some(format!("tools/call:{name}"));
    }
    Some(method.to_owned())
}

fn jsonrpc_method_is_initialize(value: &serde_json::Value) -> bool {
    value
        .get("method")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|method| method == "initialize")
}

fn is_mcp_endpoint(path: &str) -> bool {
    path == "/mcp" || path.starts_with("/mcp/")
}

fn session_invalid(failure: SessionFailure) -> Response {
    tracing::warn!(
        code = synapse_core::error_codes::HTTP_SESSION_INVALID,
        reason = ?failure,
        "HTTP MCP session rejected"
    );
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        synapse_core::error_codes::HTTP_SESSION_INVALID,
    )
        .into_response()
}

fn payload_too_large() -> Response {
    (StatusCode::PAYLOAD_TOO_LARGE, "request body too large").into_response()
}

fn cleanup_failed(error: synapse_action::ActionError) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        format!("{}: {}", error.code(), error.detail()),
    )
        .into_response()
}

fn session_registry_failed() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        "SESSION_REGISTRY_UNAVAILABLE",
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::{
        CURRENT_MCP_SESSION_ID, current_mcp_session_id, jsonrpc_action_label,
        jsonrpc_method_is_initialize, parse_idle_timeout,
    };

    #[test]
    fn initialize_detection_accepts_initialize_request_only() {
        let init = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        });
        let list = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        });
        assert!(jsonrpc_method_is_initialize(&init));
        assert!(!jsonrpc_method_is_initialize(&list));
    }

    #[test]
    fn jsonrpc_action_label_extracts_tool_call_name() {
        let value = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": "session_list", "arguments": {}}
        });
        assert_eq!(
            jsonrpc_action_label(&value).as_deref(),
            Some("tools/call:session_list")
        );
        let list = serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list"});
        assert_eq!(jsonrpc_action_label(&list).as_deref(), Some("tools/list"));
    }

    #[test]
    fn idle_timeout_parser_rejects_zero_and_invalid_values() {
        assert_eq!(parse_idle_timeout("1").unwrap_or_default(), 1);
        assert!(parse_idle_timeout("0").is_err());
        assert!(parse_idle_timeout("abc").is_err());
    }

    #[tokio::test]
    async fn current_session_id_survives_async_request_scope() {
        assert_eq!(current_mcp_session_id(), None);
        CURRENT_MCP_SESSION_ID
            .scope(Some("session-test".to_owned()), async {
                tokio::task::yield_now().await;
                assert_eq!(current_mcp_session_id().as_deref(), Some("session-test"));
            })
            .await;
        assert_eq!(current_mcp_session_id(), None);
    }
}
