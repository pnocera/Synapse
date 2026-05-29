use chrono::{DateTime, Utc};
use rmcp::ErrorData;
use serde::de::DeserializeOwned;
use serde_json::Value;
use sha2::{Digest, Sha256};
use synapse_core::error_codes;
use synapse_storage::cf;

use super::super::everquest_log::EVERQUEST_PROFILE_ID;
use super::model::{
    EverQuestWorldModelCaps, EverQuestWorldModelEvidenceBoundary, EverQuestWorldModelInspectParams,
    EverQuestWorldModelKind, EverQuestWorldModelRecordParams, EverQuestWorldModelRedaction,
    EverQuestWorldModelRetention, EverQuestWorldModelRetentionClass, EverQuestWorldModelRow,
    EverQuestWorldModelSample, EverQuestWorldModelSelectedRow, EverQuestWorldModelSourceRef,
    EverQuestWorldModelWriteMode, HARD_MAX_PAYLOAD_BYTES, INSPECT_SCAN_LIMIT, MAX_ID_BYTES,
    MAX_SAMPLE_LIMIT, MAX_SOURCE_REFS, MAX_TEXT_BYTES, NormalizedInspectParams,
    NormalizedRecordParams, SCHEMA_VERSION, len_to_u32, len_to_u64,
};
use crate::m1::mcp_error;

pub(super) fn normalize_record_params(
    params: EverQuestWorldModelRecordParams,
) -> Result<NormalizedRecordParams, ErrorData> {
    let profile_id = validate_profile_id(&params.profile_id)?;
    let row_id = validate_id("row_id", &params.row_id)?;
    let max_payload_bytes = validate_max_payload_bytes(params.max_payload_bytes)?;
    let payload_object = params
        .payload
        .as_object()
        .ok_or_else(|| params_error("payload must be a JSON object"))?;
    if payload_object.is_empty() {
        return Err(params_error("payload object must not be empty"));
    }
    if contains_forbidden_raw_payload(&params.payload) {
        return Err(params_error(
            "payload appears to contain raw chat or message text; store compact redacted fields only",
        ));
    }
    let payload_bytes = serde_json::to_vec(&params.payload).map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("encode EverQuest world-model payload: {error}"),
        )
    })?;
    if payload_bytes.len() > max_payload_bytes {
        return Err(params_error(format!(
            "payload must be <= {max_payload_bytes} bytes"
        )));
    }
    let source_refs = normalize_source_refs(params.source_refs)?;
    let row_key = world_model_row_key(&profile_id, &params.row_kind, &row_id);
    Ok(NormalizedRecordParams {
        row_kind: params.row_kind,
        row_id,
        profile_id,
        payload: params.payload,
        payload_sha256: sha256_hex(&payload_bytes),
        payload_len_bytes: len_to_u64(payload_bytes.len()),
        source_refs,
        write_mode: params.write_mode,
        retention_class: params.retention_class,
        compact_redacted: params.compact_redacted,
        max_payload_bytes,
        row_key,
    })
}

pub(super) fn normalize_inspect_params(
    params: EverQuestWorldModelInspectParams,
) -> Result<NormalizedInspectParams, ErrorData> {
    let profile_id = validate_profile_id(&params.profile_id)?;
    let sample_limit = if params.sample_limit == 0 || params.sample_limit > MAX_SAMPLE_LIMIT {
        return Err(params_error(format!(
            "sample_limit must be 1..={MAX_SAMPLE_LIMIT}"
        )));
    } else {
        params.sample_limit
    };
    let row_key = params
        .row_key
        .map(|value| normalize_world_model_row_key("row_key", &profile_id, &value))
        .transpose()?;
    Ok(NormalizedInspectParams {
        profile_id,
        row_kind: params.row_kind,
        row_key,
        sample_limit,
        include_payload: params.include_payload,
    })
}

pub(super) fn next_revision(
    params: &NormalizedRecordParams,
    existing: Option<&[u8]>,
) -> Result<(u32, Option<String>, DateTime<Utc>, bool), ErrorData> {
    let now = Utc::now();
    let Some(existing) = existing else {
        if params.write_mode == EverQuestWorldModelWriteMode::Replace {
            return Err(params_error(
                "write_mode=replace requires an existing world-model row",
            ));
        }
        return Ok((1, None, now, false));
    };
    if params.write_mode == EverQuestWorldModelWriteMode::Create {
        return Err(params_error(
            "world-model row already exists; use write_mode=replace for updates",
        ));
    }
    let existing = decode_json_row::<EverQuestWorldModelRow>(existing, "existing world-model row")?;
    Ok((
        existing.revision.saturating_add(1),
        Some(existing.payload_sha256),
        existing.created_at,
        true,
    ))
}

pub(super) fn build_world_model_row(
    params: NormalizedRecordParams,
    revision: u32,
    previous_payload_sha256: Option<String>,
    created_at: DateTime<Utc>,
    updated_existing: bool,
) -> EverQuestWorldModelRow {
    let updated_at = if updated_existing {
        Utc::now()
    } else {
        created_at
    };
    EverQuestWorldModelRow {
        schema_version: SCHEMA_VERSION,
        row_kind: "everquest_world_model".to_owned(),
        profile_id: params.profile_id,
        world_model_kind: params.row_kind,
        row_id: params.row_id,
        row_key: params.row_key,
        created_at,
        updated_at,
        revision,
        previous_payload_sha256,
        payload: params.payload,
        payload_sha256: params.payload_sha256,
        payload_len_bytes: params.payload_len_bytes,
        source_refs: params.source_refs,
        redaction: EverQuestWorldModelRedaction {
            compact_redacted: params.compact_redacted,
            raw_chat_body_persisted: false,
            raw_target_names_persisted: false,
            raw_payload_rejected: false,
        },
        retention: retention(params.retention_class),
        caps: EverQuestWorldModelCaps {
            max_payload_bytes: len_to_u64(params.max_payload_bytes),
            max_source_refs: len_to_u32(MAX_SOURCE_REFS),
            inspect_scan_limit: len_to_u32(INSPECT_SCAN_LIMIT),
        },
        evidence_boundary: evidence_boundary(),
    }
}

pub(super) fn sample_rows<I>(
    kind: &EverQuestWorldModelKind,
    rows: I,
    include_payload: bool,
) -> Result<Vec<EverQuestWorldModelSample>, ErrorData>
where
    I: Iterator<Item = (Vec<u8>, Vec<u8>)>,
{
    rows.map(|(key, value)| sample_row(kind, &key, &value, include_payload))
        .collect()
}

pub(super) fn selected_row(
    runtime: &synapse_reflex::ReflexRuntime,
    row_key: &str,
) -> Result<EverQuestWorldModelSelectedRow, ErrorData> {
    let Some(value) = runtime
        .storage_kv_row(row_key.as_bytes())
        .map_err(|error| mcp_error(error.code(), error.to_string()))?
    else {
        return Ok(EverQuestWorldModelSelectedRow {
            row_key: row_key.to_owned(),
            found: false,
            value_len_bytes: None,
            row: None,
        });
    };
    let len = len_to_u64(value.len());
    let row = decode_json_row::<EverQuestWorldModelRow>(&value, "selected world-model row")?;
    Ok(EverQuestWorldModelSelectedRow {
        row_key: row_key.to_owned(),
        found: true,
        value_len_bytes: Some(len),
        row: Some(row),
    })
}

fn sample_row(
    kind: &EverQuestWorldModelKind,
    key: &[u8],
    value: &[u8],
    include_payload: bool,
) -> Result<EverQuestWorldModelSample, ErrorData> {
    let row = decode_json_row::<EverQuestWorldModelRow>(value, "world-model sample row")?;
    Ok(EverQuestWorldModelSample {
        row_kind: kind.clone(),
        row_key: String::from_utf8_lossy(key).into_owned(),
        value_len_bytes: len_to_u64(value.len()),
        revision: Some(row.revision),
        payload_sha256: Some(row.payload_sha256),
        payload: include_payload.then_some(row.payload),
        compact_redacted: row.redaction.compact_redacted,
    })
}

fn normalize_source_refs(
    refs: Vec<EverQuestWorldModelSourceRef>,
) -> Result<Vec<EverQuestWorldModelSourceRef>, ErrorData> {
    if refs.is_empty() {
        return Err(params_error("source_refs must contain at least one ref"));
    }
    if refs.len() > MAX_SOURCE_REFS {
        return Err(params_error(format!(
            "source_refs must contain <= {MAX_SOURCE_REFS} refs"
        )));
    }
    refs.into_iter()
        .enumerate()
        .map(|(index, source)| normalize_source_ref(&format!("source_refs[{index}]"), source))
        .collect()
}

fn normalize_source_ref(
    field: &str,
    source: EverQuestWorldModelSourceRef,
) -> Result<EverQuestWorldModelSourceRef, ErrorData> {
    Ok(EverQuestWorldModelSourceRef {
        kind: normalize_required_text(&format!("{field}.kind"), &source.kind)?,
        row_key: normalize_optional_text(&format!("{field}.row_key"), source.row_key)?,
        path: normalize_optional_text(&format!("{field}.path"), source.path)?,
        start_offset: source.start_offset,
        next_offset: source.next_offset,
        content_sha256: source
            .content_sha256
            .map(|value| validate_sha256(&format!("{field}.content_sha256"), &value))
            .transpose()?,
        summary: normalize_optional_text(&format!("{field}.summary"), source.summary)?,
    })
}

fn contains_forbidden_raw_payload(value: &Value) -> bool {
    match value {
        Value::Object(map) => map
            .iter()
            .any(|(key, value)| forbidden_raw_key(key) || contains_forbidden_raw_payload(value)),
        Value::Array(values) => values.iter().any(contains_forbidden_raw_payload),
        Value::String(value) => forbidden_raw_string(value),
        _ => false,
    }
}

fn forbidden_raw_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "raw_chat",
        "raw_chat_body",
        "chat_body",
        "chat_text",
        "message_text",
        "say_text",
        "tell_text",
    ]
    .iter()
    .any(|forbidden| key.contains(forbidden))
}

fn forbidden_raw_string(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.contains("you say,") || value.contains("tells you,") || value.contains("raw chat")
}

pub(super) fn inspect_kinds(
    row_kind: Option<&EverQuestWorldModelKind>,
) -> Vec<EverQuestWorldModelKind> {
    row_kind.map_or_else(all_kinds, |kind| vec![kind.clone()])
}

fn all_kinds() -> Vec<EverQuestWorldModelKind> {
    vec![
        EverQuestWorldModelKind::Map,
        EverQuestWorldModelKind::ZoneGraph,
        EverQuestWorldModelKind::State,
        EverQuestWorldModelKind::Transition,
        EverQuestWorldModelKind::Trajectory,
        EverQuestWorldModelKind::Planner,
        EverQuestWorldModelKind::Surprise,
    ]
}

fn world_model_row_key(profile_id: &str, kind: &EverQuestWorldModelKind, row_id: &str) -> String {
    format!("{}/{row_id}", world_model_prefix(profile_id, kind))
}

pub(super) fn world_model_prefix(profile_id: &str, kind: &EverQuestWorldModelKind) -> String {
    format!("{}/{profile_id}", kind_prefix(kind))
}

const fn kind_prefix(kind: &EverQuestWorldModelKind) -> &'static str {
    match kind {
        EverQuestWorldModelKind::Map => "everquest/map/v1",
        EverQuestWorldModelKind::ZoneGraph => "everquest/zone_graph/v1",
        EverQuestWorldModelKind::State => "everquest/state/v1",
        EverQuestWorldModelKind::Transition => "everquest/transition/v1",
        EverQuestWorldModelKind::Trajectory => "everquest/trajectory/v1",
        EverQuestWorldModelKind::Planner => "everquest/planner/v1",
        EverQuestWorldModelKind::Surprise => "everquest/surprise/v1",
    }
}

fn normalize_world_model_row_key(
    field: &str,
    profile_id: &str,
    value: &str,
) -> Result<String, ErrorData> {
    let value = normalize_required_text(field, value)?;
    let allowed = all_kinds()
        .into_iter()
        .map(|kind| format!("{}/", world_model_prefix(profile_id, &kind)))
        .any(|prefix| value.starts_with(&prefix));
    if !allowed {
        return Err(params_error(format!(
            "{field} must start with an approved EverQuest world-model prefix for {profile_id}"
        )));
    }
    Ok(value)
}

const fn retention(class: EverQuestWorldModelRetentionClass) -> EverQuestWorldModelRetention {
    let (ttl_ns, pressure_preserve) = match class {
        EverQuestWorldModelRetentionClass::Strategic => (None, true),
        EverQuestWorldModelRetentionClass::Episode => {
            (Some(30 * 24 * 60 * 60 * 1_000_000_000), true)
        }
        EverQuestWorldModelRetentionClass::Scratch => (Some(24 * 60 * 60 * 1_000_000_000), false),
    };
    EverQuestWorldModelRetention {
        class,
        ttl_ns,
        pressure_preserve,
    }
}

fn evidence_boundary() -> EverQuestWorldModelEvidenceBoundary {
    EverQuestWorldModelEvidenceBoundary {
        cf_name: cf::CF_KV.to_owned(),
        source_rows_verified_by_caller: true,
        manual_fsv_required_for_runtime: true,
        is_fsv_script: false,
        note: "World-model rows are compact storage/readback surfaces. They do not replace manual gameplay FSV against physical UI, log, and storage sources of truth."
            .to_owned(),
    }
}

fn validate_profile_id(value: &str) -> Result<String, ErrorData> {
    let value = value.trim();
    if value != EVERQUEST_PROFILE_ID {
        return Err(params_error(format!(
            "profile_id must be {EVERQUEST_PROFILE_ID:?}; got {value:?}"
        )));
    }
    Ok(value.to_owned())
}

fn validate_id(field: &str, value: &str) -> Result<String, ErrorData> {
    let value = normalize_required_text(field, value)?;
    if value.len() > MAX_ID_BYTES {
        return Err(params_error(format!(
            "{field} must be <= {MAX_ID_BYTES} bytes"
        )));
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        return Err(params_error(format!(
            "{field} must contain only ASCII letters, digits, '.', '_', or '-'"
        )));
    }
    Ok(value)
}

fn validate_max_payload_bytes(value: usize) -> Result<usize, ErrorData> {
    if value == 0 || value > HARD_MAX_PAYLOAD_BYTES {
        return Err(params_error(format!(
            "max_payload_bytes must be 1..={HARD_MAX_PAYLOAD_BYTES}"
        )));
    }
    Ok(value)
}

fn normalize_required_text(field: &str, value: &str) -> Result<String, ErrorData> {
    let value = value.trim();
    if value.is_empty() {
        return Err(params_error(format!("{field} must not be empty")));
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

fn normalize_optional_text(
    field: &str,
    value: Option<String>,
) -> Result<Option<String>, ErrorData> {
    value
        .map(|value| normalize_required_text(field, &value))
        .transpose()
}

fn validate_sha256(field: &str, value: &str) -> Result<String, ErrorData> {
    let value = normalize_required_text(field, value)?;
    if value.len() != 64 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(params_error(format!(
            "{field} must be a SHA-256 hex digest"
        )));
    }
    Ok(value.to_ascii_lowercase())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_encode(&digest)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

pub(super) fn decode_json_row<T>(bytes: &[u8], label: &str) -> Result<T, ErrorData>
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

fn params_error(message: impl Into<String>) -> ErrorData {
    mcp_error(error_codes::TOOL_PARAMS_INVALID, message)
}
