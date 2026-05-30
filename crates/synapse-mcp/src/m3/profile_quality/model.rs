use std::collections::BTreeMap;

use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use synapse_core::ProfileId;

pub(super) const DEFAULT_MAX_AUDIT_ROWS: u32 = 5_000;
pub(super) const MAX_AUDIT_ROWS: u32 = 50_000;
pub(super) const DEFAULT_STALE_AFTER_NS: u64 = 24 * 60 * 60 * 1_000_000_000;
pub(super) const MAX_STALE_AFTER_NS: u64 = 30 * 24 * 60 * 60 * 1_000_000_000;
pub(super) const STORED_PREFIX_CHARS: usize = 512;
pub(super) const MAX_MANUAL_FSV_EVIDENCE_REF_CHARS: usize = 512;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileQualityRefreshParams {
    pub profile_id: ProfileId,
    #[serde(default = "default_max_audit_rows")]
    #[schemars(default = "default_max_audit_rows", range(min = 1, max = 50000))]
    pub max_audit_rows: u32,
    #[serde(default = "default_stale_after_ns")]
    #[schemars(default = "default_stale_after_ns", range(min = 1))]
    pub stale_after_ns: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_fsv_evidence_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileQualityRefreshResponse {
    pub profile_id: ProfileId,
    pub cf_name: String,
    pub key_hex: String,
    pub wrote_snapshot: bool,
    pub previous_evidence_hash: Option<String>,
    pub stored_value_len_bytes: u64,
    pub stored_value_utf8_prefix: String,
    pub snapshot: ProfileQualitySnapshot,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileQualitySnapshot {
    pub schema_version: u32,
    pub profile_id: ProfileId,
    pub profile_label: String,
    pub profile_schema_version: u32,
    pub quality_signal: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_fsv_evidence_ref: Option<String>,
    pub generated_at_ns: u64,
    pub evidence_hash: String,
    pub source: ProfileQualitySource,
    pub counts: ProfileQualityCounts,
    pub rates: ProfileQualityRates,
    pub score: ProfileQualityScore,
    pub compatibility: ProfileCompatibilitySummary,
    pub versioning: ProfileQualityVersionSummary,
    #[serde(default)]
    pub runtime_evidence: ProfileQualityRuntimeEvidence,
    #[serde(default)]
    pub reality_evidence: ProfileQualityRealityEvidence,
    pub redaction: ProfileQualityRedaction,
    pub contribution: ProfileQualityContribution,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileQualitySource {
    pub audit_cf_name: String,
    pub profile_cf_name: String,
    pub audit_rows_scanned: u64,
    pub audit_rows_decode_failed: u64,
    pub audit_rows_stale: u64,
    pub audit_rows_future: u64,
    pub audit_rows_other_profile: u64,
    pub audit_rows_profile_relevant: u64,
    pub first_relevant_audit_id: Option<String>,
    pub last_relevant_audit_id: Option<String>,
    pub first_relevant_ts_ns: Option<u64>,
    pub last_relevant_ts_ns: Option<u64>,
    pub max_audit_rows: u32,
    pub stale_after_ns: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileQualityCounts {
    pub started_rows: u64,
    pub ok_rows: u64,
    pub error_rows: u64,
    pub denied_rows: u64,
    pub unknown_status_rows: u64,
    pub quality_eligible_ok_rows: u64,
    pub quality_eligible_error_rows: u64,
    pub backend_unavailable_rows: u64,
    pub release_all_rows: u64,
    pub launch_ok_rows: u64,
    pub launch_error_rows: u64,
    pub tool_counts: BTreeMap<String, u64>,
    pub error_code_counts: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_field_names)]
pub struct ProfileQualityRates {
    pub success_rate: f64,
    pub error_rate: f64,
    pub denied_rate: f64,
    pub backend_unavailable_rate: f64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileQualityScore {
    pub score_0_100: u32,
    pub confidence_0_1: f64,
    pub wilson_success_lower_95: f64,
    pub sample_size: u64,
    pub method: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileCompatibilitySummary {
    pub foreground_match_rows: u64,
    pub active_profile_only_rows: u64,
    pub profile_mismatch_rows: u64,
    pub target_denied_rows: u64,
    pub observed_process_names: BTreeMap<String, u64>,
    pub observed_backends: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileQualityRuntimeEvidence {
    pub observation_cf_name: String,
    pub event_cf_name: String,
    pub observation_rows_scanned: u64,
    pub observation_rows_decode_failed: u64,
    pub observation_rows_stale: u64,
    pub observation_rows_future: u64,
    pub observation_rows_other_profile: u64,
    pub observation_rows_profile_relevant: u64,
    pub event_rows_scanned: u64,
    pub event_rows_decode_failed: u64,
    pub event_rows_stale: u64,
    pub event_rows_future: u64,
    pub event_rows_other_profile: u64,
    pub event_rows_profile_relevant: u64,
    pub first_relevant_observation_id: Option<String>,
    pub last_relevant_observation_id: Option<String>,
    pub first_relevant_event_id: Option<String>,
    pub last_relevant_event_id: Option<String>,
    pub last_relevant_ts_ns: Option<u64>,
    pub observed_process_names: BTreeMap<String, u64>,
    pub observed_target_ids: BTreeMap<String, u64>,
    pub observed_event_kinds: BTreeMap<String, u64>,
    pub observed_log_event_kinds: BTreeMap<String, u64>,
}

impl Default for ProfileQualityRuntimeEvidence {
    fn default() -> Self {
        Self {
            observation_cf_name: "CF_OBSERVATIONS".to_owned(),
            event_cf_name: "CF_EVENTS".to_owned(),
            observation_rows_scanned: 0,
            observation_rows_decode_failed: 0,
            observation_rows_stale: 0,
            observation_rows_future: 0,
            observation_rows_other_profile: 0,
            observation_rows_profile_relevant: 0,
            event_rows_scanned: 0,
            event_rows_decode_failed: 0,
            event_rows_stale: 0,
            event_rows_future: 0,
            event_rows_other_profile: 0,
            event_rows_profile_relevant: 0,
            first_relevant_observation_id: None,
            last_relevant_observation_id: None,
            first_relevant_event_id: None,
            last_relevant_event_id: None,
            last_relevant_ts_ns: None,
            observed_process_names: BTreeMap::new(),
            observed_target_ids: BTreeMap::new(),
            observed_event_kinds: BTreeMap::new(),
            observed_log_event_kinds: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileQualityRealityEvidence {
    pub kv_cf_name: String,
    pub reality_rows_scanned: u64,
    pub reality_rows_decode_failed: u64,
    pub reality_rows_other_profile: u64,
    pub baseline_rows: u64,
    pub head_rows: u64,
    pub delta_rows: u64,
    pub audit_rows: u64,
    pub audited_delta_rows: u64,
    pub unaudited_delta_rows: u64,
    pub in_sync_audit_rows: u64,
    pub drift_audit_rows: u64,
    pub rebase_required_rows: u64,
    pub source_unavailable_audit_rows: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_baseline_epoch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_head_epoch_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_head_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_audit_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_audit_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_audit_compared_seq_end: Option<u64>,
    pub delta_kind_counts: BTreeMap<String, u64>,
    pub delta_path_counts: BTreeMap<String, u64>,
    pub audit_drift_status_counts: BTreeMap<String, u64>,
    pub source_surface_counts: BTreeMap<String, u64>,
    pub drift_rate: f64,
    pub rebase_rate: f64,
    pub audited_delta_rate: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_op_ratio: Option<f64>,
    pub no_op_ratio_source: String,
    pub delta_first_supported: bool,
    pub full_snapshot_required: bool,
    pub calibration_source: String,
}

impl Default for ProfileQualityRealityEvidence {
    fn default() -> Self {
        Self {
            kv_cf_name: "CF_KV".to_owned(),
            reality_rows_scanned: 0,
            reality_rows_decode_failed: 0,
            reality_rows_other_profile: 0,
            baseline_rows: 0,
            head_rows: 0,
            delta_rows: 0,
            audit_rows: 0,
            audited_delta_rows: 0,
            unaudited_delta_rows: 0,
            in_sync_audit_rows: 0,
            drift_audit_rows: 0,
            rebase_required_rows: 0,
            source_unavailable_audit_rows: 0,
            latest_baseline_epoch_id: None,
            latest_head_epoch_id: None,
            latest_head_seq: None,
            latest_audit_id: None,
            latest_audit_status: None,
            latest_audit_compared_seq_end: None,
            delta_kind_counts: BTreeMap::new(),
            delta_path_counts: BTreeMap::new(),
            audit_drift_status_counts: BTreeMap::new(),
            source_surface_counts: BTreeMap::new(),
            drift_rate: 0.0,
            rebase_rate: 0.0,
            audited_delta_rate: 0.0,
            no_op_ratio: None,
            no_op_ratio_source: "not_persisted_noop_observe_delta_writes_no_delta_row".to_owned(),
            delta_first_supported: false,
            full_snapshot_required: true,
            calibration_source: "none".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileQualityVersionSummary {
    pub current_profile_schema_version: u32,
    pub rows_with_profile_schema_version: u64,
    pub current_version_rows: u64,
    pub older_version_rows: u64,
    pub newer_version_rows: u64,
    pub unknown_version_rows: u64,
    pub mixed_profile_schema_versions: bool,
    pub observed_profile_schema_versions: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileQualityRedaction {
    pub local_only: bool,
    pub snapshot_redacts_process_path: bool,
    pub snapshot_redacts_window_title: bool,
    pub retained_identifiers: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileQualityContribution {
    pub export_allowed: bool,
    pub operator_consent_required: bool,
    pub future_bundle_shape: String,
}

const fn default_max_audit_rows() -> u32 {
    DEFAULT_MAX_AUDIT_ROWS
}

const fn default_stale_after_ns() -> u64 {
    DEFAULT_STALE_AFTER_NS
}
