use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rmcp::transport::streamable_http_server::session::SessionState;
use schemars::JsonSchema;
use serde::Serialize;

const DEFAULT_STALE_AFTER_MS: u64 = 5 * 60 * 1000;

pub(crate) type SharedSessionRegistry = Arc<Mutex<SessionRegistry>>;

#[derive(Debug)]
pub(crate) struct SessionRegistry {
    stale_after_ms: u64,
    entries: BTreeMap<String, SessionRegistryEntry>,
}

#[derive(Clone, Debug)]
pub(crate) struct SessionRegistryEntry {
    pub session_id: String,
    pub transport: String,
    pub client_name: Option<String>,
    pub client_version: Option<String>,
    pub protocol_version: Option<String>,
    pub agent_kind: String,
    pub started_at_unix_ms: u64,
    pub last_seen_unix_ms: u64,
    pub closed_at_unix_ms: Option<u64>,
    pub last_action: Option<String>,
    pub last_reason_code: Option<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionRegistryRead {
    pub session_id: String,
    pub transport: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
    pub agent_kind: String,
    pub lifecycle: String,
    pub started_at_unix_ms: u64,
    pub last_seen_unix_ms: u64,
    pub last_seen_ms_ago: u64,
    pub stale_after_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_reason_code: Option<String>,
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self {
            stale_after_ms: DEFAULT_STALE_AFTER_MS,
            entries: BTreeMap::new(),
        }
    }
}

impl SessionRegistry {
    pub(crate) fn set_stale_after(&mut self, ttl: Option<Duration>) {
        self.stale_after_ms = ttl
            .map(duration_millis_u64)
            .unwrap_or(DEFAULT_STALE_AFTER_MS)
            .max(1);
    }

    pub(crate) const fn stale_after_ms(&self) -> u64 {
        self.stale_after_ms
    }

    pub(crate) fn record_initialized(
        &mut self,
        session_id: &str,
        state: &SessionState,
        transport: &str,
        now_unix_ms: u64,
    ) {
        let client_name = state.initialize_params.client_info.name.clone();
        let client_version = state.initialize_params.client_info.version.clone();
        let protocol_version = Some(format!("{:?}", state.initialize_params.protocol_version));
        let entry = self
            .entries
            .entry(session_id.to_owned())
            .or_insert_with(|| SessionRegistryEntry {
                session_id: session_id.to_owned(),
                transport: transport.to_owned(),
                client_name: None,
                client_version: None,
                protocol_version: None,
                agent_kind: "unknown".to_owned(),
                started_at_unix_ms: now_unix_ms,
                last_seen_unix_ms: now_unix_ms,
                closed_at_unix_ms: None,
                last_action: None,
                last_reason_code: None,
            });
        entry.transport = transport.to_owned();
        entry.client_name = Some(client_name.clone());
        entry.client_version = Some(client_version);
        entry.protocol_version = protocol_version;
        entry.agent_kind = infer_agent_kind(&client_name);
        entry.last_seen_unix_ms = entry.last_seen_unix_ms.max(now_unix_ms);
        entry.closed_at_unix_ms = None;
    }

    pub(crate) fn record_seen(
        &mut self,
        session_id: &str,
        action: Option<String>,
        now_unix_ms: u64,
    ) {
        let entry = self
            .entries
            .entry(session_id.to_owned())
            .or_insert_with(|| SessionRegistryEntry {
                session_id: session_id.to_owned(),
                transport: "http".to_owned(),
                client_name: None,
                client_version: None,
                protocol_version: None,
                agent_kind: "unknown".to_owned(),
                started_at_unix_ms: now_unix_ms,
                last_seen_unix_ms: now_unix_ms,
                closed_at_unix_ms: None,
                last_action: None,
                last_reason_code: None,
            });
        entry.last_seen_unix_ms = now_unix_ms;
        if let Some(action) = action {
            entry.last_action = Some(action);
            entry.last_reason_code = None;
        }
    }

    pub(crate) fn record_closed(&mut self, session_id: &str, now_unix_ms: u64) {
        let entry = self
            .entries
            .entry(session_id.to_owned())
            .or_insert_with(|| SessionRegistryEntry {
                session_id: session_id.to_owned(),
                transport: "http".to_owned(),
                client_name: None,
                client_version: None,
                protocol_version: None,
                agent_kind: "unknown".to_owned(),
                started_at_unix_ms: now_unix_ms,
                last_seen_unix_ms: now_unix_ms,
                closed_at_unix_ms: None,
                last_action: None,
                last_reason_code: None,
            });
        entry.last_seen_unix_ms = now_unix_ms;
        entry.closed_at_unix_ms = Some(now_unix_ms);
    }

    pub(crate) fn reads(&self, now_unix_ms: u64) -> Vec<SessionRegistryRead> {
        self.entries
            .values()
            .map(|entry| self.entry_read(entry, now_unix_ms))
            .collect()
    }

    pub(crate) fn entry_read(
        &self,
        entry: &SessionRegistryEntry,
        now_unix_ms: u64,
    ) -> SessionRegistryRead {
        let last_seen_ms_ago = now_unix_ms.saturating_sub(entry.last_seen_unix_ms);
        let lifecycle = if entry.closed_at_unix_ms.is_some() {
            "closed"
        } else if last_seen_ms_ago > self.stale_after_ms {
            "stale"
        } else {
            "live"
        };
        SessionRegistryRead {
            session_id: entry.session_id.clone(),
            transport: entry.transport.clone(),
            client_name: entry.client_name.clone(),
            client_version: entry.client_version.clone(),
            protocol_version: entry.protocol_version.clone(),
            agent_kind: entry.agent_kind.clone(),
            lifecycle: lifecycle.to_owned(),
            started_at_unix_ms: entry.started_at_unix_ms,
            last_seen_unix_ms: entry.last_seen_unix_ms,
            last_seen_ms_ago,
            stale_after_ms: self.stale_after_ms,
            closed_at_unix_ms: entry.closed_at_unix_ms,
            last_action: entry.last_action.clone(),
            last_reason_code: entry.last_reason_code.clone(),
        }
    }
}

pub(crate) fn unix_time_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(duration_millis_u64)
        .unwrap_or_default()
}

fn duration_millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn infer_agent_kind(client_name: &str) -> String {
    let lower = client_name.to_ascii_lowercase();
    if lower.contains("codex") {
        "codex".to_owned()
    } else if lower.contains("claude") {
        "claude".to_owned()
    } else {
        "unknown".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use rmcp::model::{ClientCapabilities, Implementation, InitializeRequestParams};

    use super::*;

    fn state(name: &str) -> SessionState {
        SessionState::new(InitializeRequestParams::new(
            ClientCapabilities::default(),
            Implementation::new(name, "0.0.0-test"),
        ))
    }

    #[test]
    fn registry_marks_live_stale_and_closed_from_heartbeats() {
        let mut registry = SessionRegistry::default();
        registry.set_stale_after(Some(Duration::from_millis(100)));
        registry.record_initialized("s1", &state("codex"), "http", 1_000);

        let live = registry.reads(1_050).remove(0);
        assert_eq!(live.lifecycle, "live");
        assert_eq!(live.agent_kind, "codex");

        let stale = registry.reads(1_200).remove(0);
        assert_eq!(stale.lifecycle, "stale");

        registry.record_closed("s1", 1_250);
        let closed = registry.reads(1_251).remove(0);
        assert_eq!(closed.lifecycle, "closed");
        assert_eq!(closed.closed_at_unix_ms, Some(1_250));
    }

    #[test]
    fn registry_initialization_never_moves_heartbeat_backwards() {
        let mut registry = SessionRegistry::default();
        registry.record_seen("s1", Some("tools/list".to_owned()), 2_000);
        registry.record_initialized("s1", &state("codex"), "http", 1_000);

        let read = registry.reads(2_001).remove(0);
        assert_eq!(read.last_seen_unix_ms, 2_000);
        assert_eq!(read.last_action.as_deref(), Some("tools/list"));
    }
}
