use chrono::{DateTime, Utc};
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::super::everquest_log::EVERQUEST_PROFILE_ID;

pub(super) const SCHEMA_VERSION: u32 = 1;
pub(super) const MAX_ID_BYTES: usize = 128;
pub(super) const MAX_TEXT_BYTES: usize = 512;
pub(super) const MAX_SOURCE_REFS: usize = 32;
pub(super) const DEFAULT_MAX_PAYLOAD_BYTES: usize = 8 * 1024;
pub(super) const HARD_MAX_PAYLOAD_BYTES: usize = 32 * 1024;
pub(super) const DEFAULT_SAMPLE_LIMIT: usize = 8;
pub(super) const MAX_SAMPLE_LIMIT: usize = 64;
pub(super) const INSPECT_SCAN_LIMIT: usize = 4096;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldModelRecordParams {
    pub row_kind: EverQuestWorldModelKind,
    pub row_id: String,
    #[serde(default = "default_profile_id")]
    pub profile_id: String,
    pub payload: Value,
    pub source_refs: Vec<EverQuestWorldModelSourceRef>,
    #[serde(default = "default_write_mode")]
    pub write_mode: EverQuestWorldModelWriteMode,
    #[serde(default = "default_retention_class")]
    pub retention_class: EverQuestWorldModelRetentionClass,
    #[serde(default = "default_compact_redacted")]
    pub compact_redacted: bool,
    #[serde(default = "default_max_payload_bytes")]
    pub max_payload_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldModelInspectParams {
    #[serde(default = "default_profile_id")]
    pub profile_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_kind: Option<EverQuestWorldModelKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_key: Option<String>,
    #[serde(default = "default_sample_limit")]
    pub sample_limit: usize,
    #[serde(default)]
    pub include_payload: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestWorldModelKind {
    Map,
    ZoneGraph,
    State,
    Transition,
    Trajectory,
    Planner,
    Surprise,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestWorldModelWriteMode {
    Create,
    Replace,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestWorldModelRetentionClass {
    Strategic,
    Episode,
    Scratch,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldModelRecordResponse {
    pub ok: bool,
    pub row_key: String,
    pub stored_value_len_bytes: u64,
    pub updated_existing: bool,
    pub row: EverQuestWorldModelRow,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldModelInspectResponse {
    pub ok: bool,
    pub profile_id: String,
    pub cf_name: String,
    pub counts: Vec<EverQuestWorldModelPrefixCount>,
    pub samples: Vec<EverQuestWorldModelSample>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<EverQuestWorldModelSelectedRow>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldModelRow {
    pub schema_version: u32,
    pub row_kind: String,
    pub profile_id: String,
    pub world_model_kind: EverQuestWorldModelKind,
    pub row_id: String,
    pub row_key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub revision: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_payload_sha256: Option<String>,
    pub payload: Value,
    pub payload_sha256: String,
    pub payload_len_bytes: u64,
    pub source_refs: Vec<EverQuestWorldModelSourceRef>,
    pub redaction: EverQuestWorldModelRedaction,
    pub retention: EverQuestWorldModelRetention,
    pub caps: EverQuestWorldModelCaps,
    pub evidence_boundary: EverQuestWorldModelEvidenceBoundary,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldModelSourceRef {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldModelRedaction {
    pub compact_redacted: bool,
    pub raw_chat_body_persisted: bool,
    pub raw_target_names_persisted: bool,
    pub raw_payload_rejected: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldModelRetention {
    pub class: EverQuestWorldModelRetentionClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_ns: Option<u64>,
    pub pressure_preserve: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldModelCaps {
    pub max_payload_bytes: u64,
    pub max_source_refs: u32,
    pub inspect_scan_limit: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldModelEvidenceBoundary {
    pub cf_name: String,
    pub source_rows_verified_by_caller: bool,
    pub manual_fsv_required_for_runtime: bool,
    pub is_fsv_script: bool,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldModelPrefixCount {
    pub row_kind: EverQuestWorldModelKind,
    pub prefix: String,
    pub row_count: u64,
    pub scan_truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldModelSample {
    pub row_kind: EverQuestWorldModelKind,
    pub row_key: String,
    pub value_len_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    pub compact_redacted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldModelSelectedRow {
    pub row_key: String,
    pub found: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_len_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row: Option<EverQuestWorldModelRow>,
}

#[derive(Clone, Debug)]
pub(super) struct NormalizedRecordParams {
    pub(super) row_kind: EverQuestWorldModelKind,
    pub(super) row_id: String,
    pub(super) profile_id: String,
    pub(super) payload: Value,
    pub(super) payload_sha256: String,
    pub(super) payload_len_bytes: u64,
    pub(super) source_refs: Vec<EverQuestWorldModelSourceRef>,
    pub(super) write_mode: EverQuestWorldModelWriteMode,
    pub(super) retention_class: EverQuestWorldModelRetentionClass,
    pub(super) compact_redacted: bool,
    pub(super) max_payload_bytes: usize,
    pub(super) row_key: String,
}

#[derive(Clone, Debug)]
pub(super) struct NormalizedInspectParams {
    pub(super) profile_id: String,
    pub(super) row_kind: Option<EverQuestWorldModelKind>,
    pub(super) row_key: Option<String>,
    pub(super) sample_limit: usize,
    pub(super) include_payload: bool,
}

fn default_profile_id() -> String {
    EVERQUEST_PROFILE_ID.to_owned()
}

const fn default_write_mode() -> EverQuestWorldModelWriteMode {
    EverQuestWorldModelWriteMode::Create
}

const fn default_retention_class() -> EverQuestWorldModelRetentionClass {
    EverQuestWorldModelRetentionClass::Strategic
}

const fn default_compact_redacted() -> bool {
    true
}

const fn default_max_payload_bytes() -> usize {
    DEFAULT_MAX_PAYLOAD_BYTES
}

const fn default_sample_limit() -> usize {
    DEFAULT_SAMPLE_LIMIT
}

pub(super) fn len_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

pub(super) fn len_to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
