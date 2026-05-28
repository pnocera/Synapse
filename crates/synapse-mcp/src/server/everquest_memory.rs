use chrono::{DateTime, Utc};
use rmcp::{ErrorData, schemars::JsonSchema};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use synapse_core::error_codes;
use synapse_storage::cf;

use super::{
    Json, Parameters, SynapseService, everquest_log::EVERQUEST_PROFILE_ID, tool, tool_router,
};
use crate::m1::mcp_error;

const RECORD_TOOL: &str = "everquest_memory_record";
const CONSULT_TOOL: &str = "everquest_memory_consult";
const SCHEMA_VERSION: u32 = 1;
const HAZARD_ROW_PREFIX: &str = "everquest/hazard_memory/v1";
const SAFE_ROW_PREFIX: &str = "everquest/safe_area_memory/v1";
const CONSULT_ROW_PREFIX: &str = "everquest/planner_consult/v1";
const DEFAULT_STALE_AFTER_SECONDS: u64 = 3600;
const DEFAULT_CONFLICT_CONFIDENCE_DELTA: f32 = 0.35;
const DEFAULT_MAX_MEMORY_ROWS: usize = 128;
const MAX_MEMORY_ROWS: usize = 512;
const MAX_ID_BYTES: usize = 128;
const MAX_TEXT_BYTES: usize = 512;
const MAX_SOURCE_REFS: usize = 32;
const MAX_MEMORY_ROW_KEYS: usize = 128;
const ACTIVE_CONFIDENCE_FLOOR: f32 = 0.50;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMemoryRecordParams {
    pub memory_id: String,
    #[serde(default = "default_profile_id")]
    pub profile_id: String,
    pub memory_type: EverQuestMemoryType,
    pub memory_kind: String,
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_short_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<EverQuestMemoryLocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    pub confidence: f32,
    #[serde(default = "default_evidence_relation")]
    pub evidence_relation: EverQuestMemoryEvidenceRelation,
    #[serde(default = "default_conflict_confidence_delta")]
    pub conflict_confidence_delta: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_state_row_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_state_generated_at: Option<DateTime<Utc>>,
    #[serde(default = "default_stale_after_seconds")]
    pub stale_after_seconds: u64,
    #[serde(default)]
    pub source_refs: Vec<EverQuestMemorySourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redacted_note: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMemoryConsultParams {
    pub candidate_id: String,
    #[serde(default = "default_profile_id")]
    pub profile_id: String,
    pub candidate_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_short_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<EverQuestMemoryLocation>,
    #[serde(default)]
    pub memory_row_keys: Vec<String>,
    #[serde(default = "default_max_memory_rows")]
    pub max_memory_rows: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestMemoryType {
    Hazard,
    SafeArea,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EverQuestMemoryEvidenceRelation {
    SupportsMemory,
    ConflictsWithMemory,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMemoryRecordResponse {
    pub ok: bool,
    pub row_key: String,
    pub duplicate_of_prior_row: bool,
    pub stored_value_len_bytes: u64,
    pub memory: EverQuestWorldMemoryRow,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMemoryConsultResponse {
    pub ok: bool,
    pub row_key: String,
    pub stored_value_len_bytes: u64,
    pub consult: EverQuestPlannerConsultRow,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestWorldMemoryRow {
    pub schema_version: u32,
    pub row_kind: String,
    pub profile_id: String,
    pub memory_id: String,
    pub row_key: String,
    pub memory_type: EverQuestMemoryType,
    pub memory_kind: String,
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_short_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<EverQuestMemoryLocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    pub severity: String,
    pub confidence: f32,
    pub active_for_planning: bool,
    pub planning_status: String,
    pub source_status: String,
    pub stale_source_state: bool,
    pub duplicate_of_prior_row: bool,
    pub conflict_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_confidence: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_state_row_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_state_generated_at: Option<DateTime<Utc>>,
    pub source_refs: Vec<EverQuestMemorySourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redacted_note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub redaction: EverQuestMemoryRedaction,
    pub evidence_boundary: EverQuestMemoryEvidenceBoundary,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPlannerConsultRow {
    pub schema_version: u32,
    pub row_kind: String,
    pub profile_id: String,
    pub candidate_id: String,
    pub candidate: EverQuestPlannerCandidate,
    pub generated_at: DateTime<Utc>,
    pub decision: String,
    pub reason: String,
    pub matched_hazards: Vec<EverQuestPlannerMemoryMatch>,
    pub matched_safe_areas: Vec<EverQuestPlannerMemoryMatch>,
    pub scanned_memory_count: usize,
    pub ignored_memory_count: usize,
    pub evidence_boundary: EverQuestMemoryEvidenceBoundary,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPlannerCandidate {
    pub candidate_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone_short_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<EverQuestMemoryLocation>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPlannerMemoryMatch {
    pub row_key: String,
    pub memory_id: String,
    pub memory_type: EverQuestMemoryType,
    pub memory_kind: String,
    pub subject: String,
    pub confidence: f32,
    pub severity: String,
    pub match_reasons: Vec<String>,
}

#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMemoryLocation {
    pub map_x: f64,
    pub map_y: f64,
    pub map_z: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMemorySourceRef {
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
    pub log_timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMemoryRedaction {
    pub compact_redacted: bool,
    pub raw_chat_body_persisted: bool,
    pub source_hash_present: bool,
    pub note: String,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestMemoryEvidenceBoundary {
    pub supports_planning: bool,
    pub manual_fsv_required_for_runtime: bool,
    pub is_fsv: bool,
    pub redacted: bool,
    pub note: String,
}

#[derive(Clone, Debug)]
struct NormalizedRecord {
    params: EverQuestMemoryRecordParams,
    row_key: String,
}

#[tool_router(router = everquest_memory_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Persist one compact EverQuest hazard or safe-area memory row with source refs, stale/conflict handling, and exact CF_KV readback"
    )]
    pub async fn everquest_memory_record(
        &self,
        params: Parameters<EverQuestMemoryRecordParams>,
    ) -> Result<Json<EverQuestMemoryRecordResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = RECORD_TOOL,
            "tool.invocation kind=everquest_memory_record"
        );
        let normalized = normalize_record_params(params.0)?;
        let (memory, stored_value_len_bytes) = self.persist_memory_row(normalized)?;
        Ok(Json(EverQuestMemoryRecordResponse {
            ok: true,
            row_key: memory.row_key.clone(),
            duplicate_of_prior_row: memory.duplicate_of_prior_row,
            stored_value_len_bytes,
            memory,
        }))
    }

    #[tool(
        description = "Consult persisted EverQuest hazard and safe-area memories for one candidate action, write a compact planner consult row, and read it back"
    )]
    pub async fn everquest_memory_consult(
        &self,
        params: Parameters<EverQuestMemoryConsultParams>,
    ) -> Result<Json<EverQuestMemoryConsultResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = CONSULT_TOOL,
            "tool.invocation kind=everquest_memory_consult"
        );
        let params = normalize_consult_params(params.0)?;
        let rows = self.read_memory_rows(&params)?;
        let consult = consult_row_from_rows(&params, &rows);
        let key = consult_row_key(&params.profile_id, &params.candidate_id);
        let (consult, stored_value_len_bytes) =
            self.persist_memory_kv_json(&key, &consult, "EverQuest planner consult row")?;
        Ok(Json(EverQuestMemoryConsultResponse {
            ok: true,
            row_key: key,
            stored_value_len_bytes,
            consult,
        }))
    }
}

impl SynapseService {
    fn persist_memory_row(
        &self,
        normalized: NormalizedRecord,
    ) -> Result<(EverQuestWorldMemoryRow, u64), ErrorData> {
        let existing = self.read_optional_memory_row(&normalized.row_key)?;
        let row = memory_row_from_params(normalized, existing.as_ref());
        self.persist_memory_kv_json(&row.row_key, &row, "EverQuest hazard/safe memory row")
    }

    fn read_optional_memory_row(
        &self,
        key: &str,
    ) -> Result<Option<EverQuestWorldMemoryRow>, ErrorData> {
        let stored = {
            let runtime = self.reflex_runtime()?;
            let runtime = runtime.lock().map_err(|_error| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    "reflex runtime lock poisoned while reading EverQuest memory row",
                )
            })?;
            runtime
                .storage_kv_row(key.as_bytes())
                .map_err(|error| mcp_error(error.code(), error.to_string()))?
        };
        stored
            .map(|stored| decode_json_row::<EverQuestWorldMemoryRow>(&stored, key))
            .transpose()
    }

    fn read_memory_rows(
        &self,
        params: &EverQuestMemoryConsultParams,
    ) -> Result<Vec<EverQuestWorldMemoryRow>, ErrorData> {
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "reflex runtime lock poisoned while reading EverQuest memory rows",
            )
        })?;
        let mut rows = Vec::new();
        if params.memory_row_keys.is_empty() {
            for prefix in [
                format!("{HAZARD_ROW_PREFIX}/{}/", params.profile_id),
                format!("{SAFE_ROW_PREFIX}/{}/", params.profile_id),
            ] {
                for (_key, value) in runtime
                    .storage_cf_prefix_rows(cf::CF_KV, prefix.as_bytes(), params.max_memory_rows)
                    .map_err(|error| mcp_error(error.code(), error.to_string()))?
                {
                    rows.push(decode_json_row::<EverQuestWorldMemoryRow>(
                        &value,
                        "EverQuest memory prefix row",
                    )?);
                    if rows.len() >= params.max_memory_rows {
                        break;
                    }
                }
            }
        } else {
            for key in &params.memory_row_keys {
                let value = runtime
                    .storage_kv_row(key.as_bytes())
                    .map_err(|error| mcp_error(error.code(), error.to_string()))?
                    .ok_or_else(|| {
                        mcp_error(
                            error_codes::STORAGE_READ_FAILED,
                            format!("EverQuest memory row missing: {key}"),
                        )
                    })?;
                rows.push(decode_json_row::<EverQuestWorldMemoryRow>(
                    &value,
                    "EverQuest memory keyed row",
                )?);
            }
        }
        drop(runtime);
        Ok(rows)
    }

    fn persist_memory_kv_json<T>(
        &self,
        key: &str,
        row: &T,
        label: &str,
    ) -> Result<(T, u64), ErrorData>
    where
        T: DeserializeOwned + Serialize,
    {
        let encoded = serde_json::to_vec(row).map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("encode {label}: {error}"),
            )
        })?;
        let stored = {
            let runtime = self.reflex_runtime()?;
            let runtime = runtime.lock().map_err(|_error| {
                mcp_error(
                    error_codes::TOOL_INTERNAL_ERROR,
                    format!("reflex runtime lock poisoned while writing {label}"),
                )
            })?;
            runtime
                .storage_put_kv_rows(vec![(key.as_bytes().to_vec(), encoded)])
                .map_err(|error| {
                    mcp_error(
                        error_codes::STORAGE_WRITE_FAILED,
                        format!("write {label}: {error}"),
                    )
                })?;
            runtime
                .storage_kv_row(key.as_bytes())
                .map_err(|error| {
                    mcp_error(
                        error_codes::STORAGE_READ_FAILED,
                        format!("read {label} after write: {error}"),
                    )
                })?
                .ok_or_else(|| {
                    mcp_error(
                        error_codes::STORAGE_READ_FAILED,
                        format!("{label} missing after write"),
                    )
                })?
        };
        let readback = decode_json_row::<T>(&stored, label)?;
        Ok((readback, len_to_u64(stored.len())))
    }
}

fn normalize_record_params(
    params: EverQuestMemoryRecordParams,
) -> Result<NormalizedRecord, ErrorData> {
    let profile_id = validate_everquest_profile_id(&params.profile_id)?;
    let memory_id = validate_id("memory_id", &params.memory_id)?;
    let memory_kind = normalize_required_text("memory_kind", &params.memory_kind)?;
    let subject = normalize_required_text("subject", &params.subject)?;
    validate_unit_interval("confidence", params.confidence)?;
    validate_unit_interval(
        "conflict_confidence_delta",
        params.conflict_confidence_delta,
    )?;
    let zone_short_name = normalize_optional_id("zone_short_name", params.zone_short_name)?;
    let radius = params.radius.map(validate_radius).transpose()?;
    if params.stale_after_seconds == 0 {
        return Err(params_error("stale_after_seconds must be >= 1"));
    }
    let source_state_row_key = params
        .source_state_row_key
        .map(|value| normalize_required_text("source_state_row_key", &value))
        .transpose()?;
    let source_refs = normalize_source_refs(params.source_refs)?;
    if source_refs.is_empty() {
        return Err(params_error(
            "source_refs must contain at least one physical SoT reference",
        ));
    }
    let redacted_note = params
        .redacted_note
        .map(|value| normalize_required_text("redacted_note", &value))
        .transpose()?;
    let row_key = memory_row_key(&profile_id, &params.memory_type, &memory_id);
    Ok(NormalizedRecord {
        params: EverQuestMemoryRecordParams {
            profile_id,
            memory_id,
            memory_type: params.memory_type,
            memory_kind,
            subject,
            zone_short_name,
            location: params.location,
            radius,
            severity: params
                .severity
                .map(|value| normalize_required_text("severity", &value))
                .transpose()?,
            confidence: params.confidence,
            evidence_relation: params.evidence_relation,
            conflict_confidence_delta: params.conflict_confidence_delta,
            source_state_row_key,
            source_state_generated_at: params.source_state_generated_at,
            stale_after_seconds: params.stale_after_seconds,
            source_refs,
            redacted_note,
        },
        row_key,
    })
}

fn normalize_consult_params(
    params: EverQuestMemoryConsultParams,
) -> Result<EverQuestMemoryConsultParams, ErrorData> {
    let candidate_id = validate_id("candidate_id", &params.candidate_id)?;
    let profile_id = validate_everquest_profile_id(&params.profile_id)?;
    let candidate_kind = normalize_required_text("candidate_kind", &params.candidate_kind)?;
    let target = params
        .target
        .map(|value| normalize_required_text("target", &value))
        .transpose()?;
    let zone_short_name = normalize_optional_id("zone_short_name", params.zone_short_name)?;
    if params.max_memory_rows == 0 || params.max_memory_rows > MAX_MEMORY_ROWS {
        return Err(params_error(format!(
            "max_memory_rows must be between 1 and {MAX_MEMORY_ROWS}"
        )));
    }
    if params.memory_row_keys.len() > MAX_MEMORY_ROW_KEYS {
        return Err(params_error(format!(
            "memory_row_keys must contain <= {MAX_MEMORY_ROW_KEYS} keys"
        )));
    }
    let memory_row_keys = params
        .memory_row_keys
        .into_iter()
        .enumerate()
        .map(|(index, key)| normalize_required_text(&format!("memory_row_keys[{index}]"), &key))
        .collect::<Result<Vec<_>, ErrorData>>()?;
    Ok(EverQuestMemoryConsultParams {
        candidate_id,
        profile_id,
        candidate_kind,
        target,
        zone_short_name,
        location: params.location,
        memory_row_keys,
        max_memory_rows: params.max_memory_rows,
    })
}

fn memory_row_from_params(
    normalized: NormalizedRecord,
    existing: Option<&EverQuestWorldMemoryRow>,
) -> EverQuestWorldMemoryRow {
    let now = Utc::now();
    let params = normalized.params;
    let stale_source_state =
        is_stale_source(params.source_state_generated_at, params.stale_after_seconds);
    let duplicate_of_prior_row = existing.is_some();
    let prior_confidence = existing.map(|row| row.confidence);
    let conflict_count = existing.map_or(0, |row| row.conflict_count)
        + u32::from(
            params.evidence_relation == EverQuestMemoryEvidenceRelation::ConflictsWithMemory,
        );
    let base_confidence = if let Some(existing) = existing
        && params.evidence_relation == EverQuestMemoryEvidenceRelation::ConflictsWithMemory
    {
        (existing.confidence - params.conflict_confidence_delta).max(0.0)
    } else {
        params.confidence
    };
    let confidence = if stale_source_state {
        base_confidence.min(0.25)
    } else {
        base_confidence
    };
    let redaction = redaction_for_refs(&params.source_refs);
    let active_for_planning = confidence >= ACTIVE_CONFIDENCE_FLOOR
        && !stale_source_state
        && params.evidence_relation == EverQuestMemoryEvidenceRelation::SupportsMemory;
    let planning_status = planning_status_for(
        active_for_planning,
        stale_source_state,
        &params.evidence_relation,
        duplicate_of_prior_row,
    );
    let severity = params
        .severity
        .unwrap_or_else(|| default_severity(&params.memory_type));
    EverQuestWorldMemoryRow {
        schema_version: SCHEMA_VERSION,
        row_kind: match params.memory_type {
            EverQuestMemoryType::Hazard => "everquest_hazard_memory",
            EverQuestMemoryType::SafeArea => "everquest_safe_area_memory",
        }
        .to_owned(),
        profile_id: params.profile_id,
        memory_id: params.memory_id,
        row_key: normalized.row_key,
        memory_type: params.memory_type,
        memory_kind: params.memory_kind,
        subject: params.subject,
        zone_short_name: params.zone_short_name,
        location: params.location,
        radius: params.radius,
        severity,
        confidence,
        active_for_planning,
        planning_status,
        source_status: if stale_source_state {
            "stale_source_state"
        } else {
            "fresh_or_not_time_bound"
        }
        .to_owned(),
        stale_source_state,
        duplicate_of_prior_row,
        conflict_count,
        prior_confidence,
        source_state_row_key: params.source_state_row_key,
        source_state_generated_at: params.source_state_generated_at,
        source_refs: params.source_refs,
        redacted_note: params.redacted_note,
        created_at: existing.map_or(now, |row| row.created_at),
        updated_at: now,
        redaction,
        evidence_boundary: evidence_boundary(),
    }
}

fn consult_row_from_rows(
    params: &EverQuestMemoryConsultParams,
    rows: &[EverQuestWorldMemoryRow],
) -> EverQuestPlannerConsultRow {
    let candidate = EverQuestPlannerCandidate {
        candidate_kind: params.candidate_kind.clone(),
        target: params.target.clone(),
        zone_short_name: params.zone_short_name.clone(),
        location: params.location.clone(),
    };
    let mut ignored_memory_count = 0_usize;
    let mut matched_hazards = Vec::new();
    let mut matched_safe_areas = Vec::new();
    for row in rows {
        if row.profile_id != params.profile_id || !row.active_for_planning {
            ignored_memory_count = ignored_memory_count.saturating_add(1);
            continue;
        }
        let match_reasons = match_reasons(&candidate, row);
        if match_reasons.is_empty() {
            continue;
        }
        let matched = EverQuestPlannerMemoryMatch {
            row_key: row.row_key.clone(),
            memory_id: row.memory_id.clone(),
            memory_type: row.memory_type.clone(),
            memory_kind: row.memory_kind.clone(),
            subject: row.subject.clone(),
            confidence: row.confidence,
            severity: row.severity.clone(),
            match_reasons,
        };
        match row.memory_type {
            EverQuestMemoryType::Hazard => matched_hazards.push(matched),
            EverQuestMemoryType::SafeArea => matched_safe_areas.push(matched),
        }
    }
    let (decision, reason) = decision_for(&candidate, &matched_hazards, &matched_safe_areas);
    EverQuestPlannerConsultRow {
        schema_version: SCHEMA_VERSION,
        row_kind: "everquest_planner_consult".to_owned(),
        profile_id: params.profile_id.clone(),
        candidate_id: params.candidate_id.clone(),
        candidate,
        generated_at: Utc::now(),
        decision,
        reason,
        matched_hazards,
        matched_safe_areas,
        scanned_memory_count: rows.len(),
        ignored_memory_count,
        evidence_boundary: evidence_boundary(),
    }
}

fn decision_for(
    candidate: &EverQuestPlannerCandidate,
    matched_hazards: &[EverQuestPlannerMemoryMatch],
    matched_safe_areas: &[EverQuestPlannerMemoryMatch],
) -> (String, String) {
    if candidate.zone_short_name.is_none()
        && candidate.location.is_none()
        && candidate.target.is_none()
    {
        return (
            "abstain_state_unknown".to_owned(),
            "candidate has no target, zone, or location for memory matching".to_owned(),
        );
    }
    if !matched_hazards.is_empty() {
        return (
            "avoid".to_owned(),
            format!(
                "matched {} active hazard memory row(s); planner must avoid or request safer probe",
                matched_hazards.len()
            ),
        );
    }
    if !matched_safe_areas.is_empty() {
        return (
            "allow_with_safe_memory".to_owned(),
            format!(
                "matched {} active safe-area memory row(s) and no active hazards",
                matched_safe_areas.len()
            ),
        );
    }
    (
        "allow_no_matching_hazard".to_owned(),
        "no active hazard memory matched the candidate".to_owned(),
    )
}

fn match_reasons(
    candidate: &EverQuestPlannerCandidate,
    row: &EverQuestWorldMemoryRow,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if let (Some(candidate_target), subject) = (candidate.target.as_deref(), row.subject.as_str())
        && same_label(candidate_target, subject)
    {
        reasons.push("target_subject_match".to_owned());
    }
    if let (Some(candidate_zone), Some(row_zone)) = (
        candidate.zone_short_name.as_deref(),
        row.zone_short_name.as_deref(),
    ) && same_label(candidate_zone, row_zone)
    {
        reasons.push("zone_match".to_owned());
    }
    if let (Some(candidate_location), Some(row_location), Some(radius)) =
        (&candidate.location, &row.location, row.radius)
        && distance(candidate_location, row_location) <= radius
    {
        reasons.push("location_radius_match".to_owned());
    }
    reasons
}

fn planning_status_for(
    active_for_planning: bool,
    stale_source_state: bool,
    relation: &EverQuestMemoryEvidenceRelation,
    duplicate_of_prior_row: bool,
) -> String {
    if stale_source_state {
        return "stale_source_needs_refresh".to_owned();
    }
    if relation == &EverQuestMemoryEvidenceRelation::ConflictsWithMemory {
        return "downgraded_by_conflict".to_owned();
    }
    if active_for_planning && duplicate_of_prior_row {
        return "active_duplicate_readback".to_owned();
    }
    if active_for_planning {
        return "active".to_owned();
    }
    "below_active_confidence_floor".to_owned()
}

fn is_stale_source(source_generated_at: Option<DateTime<Utc>>, stale_after_seconds: u64) -> bool {
    source_generated_at.is_some_and(|generated_at| {
        let age = Utc::now().signed_duration_since(generated_at);
        age.num_seconds() > i64::try_from(stale_after_seconds).unwrap_or(i64::MAX)
    })
}

fn redaction_for_refs(existing_refs: &[EverQuestMemorySourceRef]) -> EverQuestMemoryRedaction {
    let source_hash_present = existing_refs
        .iter()
        .any(|source| source.content_sha256.is_some());
    EverQuestMemoryRedaction {
        compact_redacted: true,
        raw_chat_body_persisted: false,
        source_hash_present,
        note: "Memory rows store compact source references, hashes, and redacted summaries only; raw chat bodies are rejected by the closed schema and not persisted."
            .to_owned(),
    }
}

fn evidence_boundary() -> EverQuestMemoryEvidenceBoundary {
    EverQuestMemoryEvidenceBoundary {
        supports_planning: true,
        manual_fsv_required_for_runtime: true,
        is_fsv: false,
        redacted: true,
        note: "Hazard/safe memories guide planner candidates; manual FSV against physical EQ logs/UI/storage still gates runtime claims."
            .to_owned(),
    }
}

fn normalize_source_refs(
    refs: Vec<EverQuestMemorySourceRef>,
) -> Result<Vec<EverQuestMemorySourceRef>, ErrorData> {
    if refs.len() > MAX_SOURCE_REFS {
        return Err(params_error(format!(
            "source_refs must contain <= {MAX_SOURCE_REFS} refs"
        )));
    }
    refs.into_iter()
        .enumerate()
        .map(|(index, source)| {
            let kind =
                normalize_required_text(&format!("source_refs[{index}].kind"), &source.kind)?;
            let row_key = source
                .row_key
                .map(|value| {
                    normalize_required_text(&format!("source_refs[{index}].row_key"), &value)
                })
                .transpose()?;
            let path = source
                .path
                .map(|value| normalize_required_text(&format!("source_refs[{index}].path"), &value))
                .transpose()?;
            let log_timestamp = source
                .log_timestamp
                .map(|value| {
                    normalize_required_text(&format!("source_refs[{index}].log_timestamp"), &value)
                })
                .transpose()?;
            let content_sha256 = source
                .content_sha256
                .map(|value| {
                    validate_sha256(&format!("source_refs[{index}].content_sha256"), &value)
                })
                .transpose()?;
            let summary = source
                .summary
                .map(|value| {
                    normalize_required_text(&format!("source_refs[{index}].summary"), &value)
                })
                .transpose()?;
            Ok(EverQuestMemorySourceRef {
                kind,
                row_key,
                path,
                start_offset: source.start_offset,
                next_offset: source.next_offset,
                log_timestamp,
                content_sha256,
                summary,
            })
        })
        .collect()
}

fn validate_everquest_profile_id(value: &str) -> Result<String, ErrorData> {
    let profile_id = normalize_required_text("profile_id", value)?;
    if profile_id != EVERQUEST_PROFILE_ID {
        return Err(params_error(format!(
            "profile_id must be {EVERQUEST_PROFILE_ID:?}; got {profile_id:?}"
        )));
    }
    Ok(profile_id)
}

fn validate_id(field: &str, value: &str) -> Result<String, ErrorData> {
    let value = value.trim();
    if value.is_empty() {
        return Err(params_error(format!("{field} must not be empty")));
    }
    if value.len() > MAX_ID_BYTES {
        return Err(params_error(format!(
            "{field} must be <= {MAX_ID_BYTES} bytes"
        )));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(params_error(format!(
            "{field} may contain only ASCII letters, digits, '.', '_', and '-'"
        )));
    }
    Ok(value.to_owned())
}

fn normalize_optional_id(field: &str, value: Option<String>) -> Result<Option<String>, ErrorData> {
    value.map(|value| validate_id(field, &value)).transpose()
}

fn normalize_required_text(field: &str, value: &str) -> Result<String, ErrorData> {
    let value = value.trim();
    if value.is_empty() {
        return Err(params_error(format!(
            "{field} must not be empty when present"
        )));
    }
    if value.len() > MAX_TEXT_BYTES {
        return Err(params_error(format!(
            "{field} must be <= {MAX_TEXT_BYTES} bytes"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(params_error(format!(
            "{field} must not contain control characters"
        )));
    }
    Ok(value.to_owned())
}

fn validate_sha256(field: &str, value: &str) -> Result<String, ErrorData> {
    let value = value.trim();
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(params_error(format!(
            "{field} must be a 64-character hex SHA-256"
        )));
    }
    Ok(value.to_ascii_lowercase())
}

fn validate_unit_interval(field: &str, value: f32) -> Result<(), ErrorData> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(params_error(format!(
            "{field} must be a finite value between 0.0 and 1.0"
        )));
    }
    Ok(())
}

fn validate_radius(radius: f64) -> Result<f64, ErrorData> {
    if !radius.is_finite() || !(0.0..=10_000.0).contains(&radius) {
        return Err(params_error(
            "radius must be a finite value between 0.0 and 10000.0",
        ));
    }
    Ok(radius)
}

fn decode_json_row<T>(bytes: &[u8], label: &str) -> Result<T, ErrorData>
where
    T: DeserializeOwned,
{
    serde_json::from_slice::<T>(bytes).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_CORRUPTED,
            format!("decode {label}: {error}"),
        )
    })
}

fn memory_row_key(profile_id: &str, memory_type: &EverQuestMemoryType, memory_id: &str) -> String {
    let prefix = match memory_type {
        EverQuestMemoryType::Hazard => HAZARD_ROW_PREFIX,
        EverQuestMemoryType::SafeArea => SAFE_ROW_PREFIX,
    };
    format!("{prefix}/{profile_id}/{memory_id}")
}

fn consult_row_key(profile_id: &str, candidate_id: &str) -> String {
    format!("{CONSULT_ROW_PREFIX}/{profile_id}/{candidate_id}")
}

fn default_severity(memory_type: &EverQuestMemoryType) -> String {
    match memory_type {
        EverQuestMemoryType::Hazard => "high".to_owned(),
        EverQuestMemoryType::SafeArea => "supportive".to_owned(),
    }
}

fn same_label(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn distance(left: &EverQuestMemoryLocation, right: &EverQuestMemoryLocation) -> f64 {
    let dx = left.map_x - right.map_x;
    let dy = left.map_y - right.map_y;
    let dz = left.map_z - right.map_z;
    dx.mul_add(dx, dy.mul_add(dy, dz * dz)).sqrt()
}

fn len_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn params_error(message: impl Into<String>) -> ErrorData {
    mcp_error(error_codes::TOOL_PARAMS_INVALID, message)
}

fn default_profile_id() -> String {
    EVERQUEST_PROFILE_ID.to_owned()
}

const fn default_stale_after_seconds() -> u64 {
    DEFAULT_STALE_AFTER_SECONDS
}

const fn default_conflict_confidence_delta() -> f32 {
    DEFAULT_CONFLICT_CONFIDENCE_DELTA
}

const fn default_max_memory_rows() -> usize {
    DEFAULT_MAX_MEMORY_ROWS
}

const fn default_evidence_relation() -> EverQuestMemoryEvidenceRelation {
    EverQuestMemoryEvidenceRelation::SupportsMemory
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn source() -> EverQuestMemorySourceRef {
        EverQuestMemorySourceRef {
            kind: "everquest_outcome_event".to_owned(),
            row_key: Some("everquest/outcome_event/v1/everquest.live/abc".to_owned()),
            path: Some("C:/eq/Logs/eqlog_Thenumberone_frostreaver.txt".to_owned()),
            start_offset: Some(10),
            next_offset: Some(20),
            log_timestamp: Some("2026-05-28T16:18:36".to_owned()),
            content_sha256: Some("a".repeat(64)),
            summary: Some("pyre beetle hit you".to_owned()),
        }
    }

    fn hazard_params(memory_id: &str) -> EverQuestMemoryRecordParams {
        EverQuestMemoryRecordParams {
            memory_id: memory_id.to_owned(),
            profile_id: EVERQUEST_PROFILE_ID.to_owned(),
            memory_type: EverQuestMemoryType::Hazard,
            memory_kind: "high_risk_target".to_owned(),
            subject: "pyre beetle".to_owned(),
            zone_short_name: Some("nektulos".to_owned()),
            location: Some(EverQuestMemoryLocation {
                map_x: 10.0,
                map_y: 20.0,
                map_z: 0.0,
            }),
            radius: Some(50.0),
            severity: None,
            confidence: 0.9,
            evidence_relation: EverQuestMemoryEvidenceRelation::SupportsMemory,
            conflict_confidence_delta: DEFAULT_CONFLICT_CONFIDENCE_DELTA,
            source_state_row_key: Some("everquest/current_state/v1/everquest.live".to_owned()),
            source_state_generated_at: Some(Utc::now()),
            stale_after_seconds: DEFAULT_STALE_AFTER_SECONDS,
            source_refs: vec![source()],
            redacted_note: Some("level 1 wizard should avoid this target".to_owned()),
        }
    }

    #[test]
    fn memory_row_key_uses_type_prefix() -> Result<(), ErrorData> {
        let normalized = normalize_record_params(hazard_params("pyre-beetle"))?;
        assert_eq!(
            normalized.row_key,
            "everquest/hazard_memory/v1/everquest.live/pyre-beetle"
        );
        Ok(())
    }

    #[test]
    fn stale_source_caps_confidence_and_disables_planning() -> Result<(), ErrorData> {
        let mut params = hazard_params("stale");
        params.source_state_generated_at = Some(Utc::now() - Duration::seconds(7200));
        params.stale_after_seconds = 60;
        let normalized = normalize_record_params(params)?;
        let row = memory_row_from_params(normalized, None);
        assert!(row.stale_source_state);
        assert_eq!(row.planning_status, "stale_source_needs_refresh");
        assert!(!row.active_for_planning);
        assert!(row.confidence <= 0.25);
        Ok(())
    }

    #[test]
    fn conflicting_evidence_downgrades_existing_hazard() -> Result<(), ErrorData> {
        let normalized = normalize_record_params(hazard_params("conflict"))?;
        let existing = memory_row_from_params(normalized, None);
        let mut params = hazard_params("conflict");
        params.evidence_relation = EverQuestMemoryEvidenceRelation::ConflictsWithMemory;
        params.conflict_confidence_delta = 0.6;
        let normalized = normalize_record_params(params)?;
        let downgraded = memory_row_from_params(normalized, Some(&existing));
        assert_eq!(downgraded.planning_status, "downgraded_by_conflict");
        assert_eq!(downgraded.prior_confidence, Some(existing.confidence));
        assert!(downgraded.confidence < ACTIVE_CONFIDENCE_FLOOR);
        assert!(!downgraded.active_for_planning);
        Ok(())
    }

    #[test]
    fn consult_avoids_matching_active_hazard() -> Result<(), ErrorData> {
        let normalized = normalize_record_params(hazard_params("pyre-beetle"))?;
        let row = memory_row_from_params(normalized, None);
        let params = EverQuestMemoryConsultParams {
            candidate_id: "candidate-1".to_owned(),
            profile_id: EVERQUEST_PROFILE_ID.to_owned(),
            candidate_kind: "combat_engage".to_owned(),
            target: Some("Pyre Beetle".to_owned()),
            zone_short_name: Some("nektulos".to_owned()),
            location: None,
            memory_row_keys: Vec::new(),
            max_memory_rows: DEFAULT_MAX_MEMORY_ROWS,
        };
        let consult = consult_row_from_rows(&params, &[row]);
        assert_eq!(consult.decision, "avoid");
        assert_eq!(consult.matched_hazards.len(), 1);
        assert!(
            consult.matched_hazards[0]
                .match_reasons
                .contains(&"target_subject_match".to_owned())
        );
        Ok(())
    }
}
