use std::time::Duration;

use anyhow::{Context, bail};
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use rmcp::transport::streamable_http_server::session::local::SessionConfig;

const SESSION_IDLE_TIMEOUT_ENV: &str = "SYNAPSE_HTTP_SESSION_IDLE_TIMEOUT_SECS";
const DEFAULT_SESSION_IDLE_TIMEOUT_SECS: u64 = 30 * 60;
const MAX_MCP_REQUEST_BYTES: usize = 1024 * 1024;
const SESSION_ID_HEADER: &str = "Mcp-Session-Id";

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum SessionFailure {
    Missing,
    UnknownOrExpired,
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

pub(super) async fn require_mcp_session(request: Request<Body>, next: Next) -> Response {
    if !is_mcp_endpoint(request.uri().path()) {
        return next.run(request).await;
    }
    let request = match enforce_session_header(request).await {
        Ok(request) => request,
        Err(response) => return response,
    };
    let response = next.run(request).await;
    if response.status() == StatusCode::NOT_FOUND {
        return session_invalid(SessionFailure::UnknownOrExpired);
    }
    response
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
    request
        .headers()
        .get(SESSION_ID_HEADER)
        .is_some_and(|value| value.to_str().is_ok_and(|text| !text.trim().is_empty()))
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

#[cfg(test)]
mod tests {
    use super::{jsonrpc_method_is_initialize, parse_idle_timeout};

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
    fn idle_timeout_parser_rejects_zero_and_invalid_values() {
        assert_eq!(parse_idle_timeout("1").unwrap_or_default(), 1);
        assert!(parse_idle_timeout("0").is_err());
        assert!(parse_idle_timeout("abc").is_err());
    }
}
