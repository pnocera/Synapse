use synapse_core::error_codes;

use super::super::everquest_log::EVERQUEST_PROFILE_ID;
use super::model::{
    DEFAULT_MAX_PAYLOAD_BYTES, EverQuestWorldModelKind, EverQuestWorldModelRecordParams,
    EverQuestWorldModelRetentionClass, EverQuestWorldModelSourceRef, EverQuestWorldModelWriteMode,
};
use super::validation::normalize_record_params;

#[test]
fn rejects_raw_chat_payload_key() {
    let mut params = base_params();
    params.payload = serde_json::json!({"raw_chat_body": "redacted by test"});
    let error = normalize_record_params(params).unwrap_err();
    assert_eq!(
        error.data.as_ref().and_then(|data| data.get("code")),
        Some(&serde_json::json!(error_codes::TOOL_PARAMS_INVALID))
    );
}

#[test]
fn rejects_payload_over_declared_cap() {
    let mut params = base_params();
    params.max_payload_bytes = 8;
    let error = normalize_record_params(params).unwrap_err();
    assert_eq!(
        error.data.as_ref().and_then(|data| data.get("code")),
        Some(&serde_json::json!(error_codes::TOOL_PARAMS_INVALID))
    );
}

#[test]
fn builds_approved_state_row_key() {
    let params = normalize_record_params(base_params()).unwrap();
    assert_eq!(
        params.row_key,
        "everquest/state/v1/everquest.live/issue513-state-01"
    );
}

fn base_params() -> EverQuestWorldModelRecordParams {
    EverQuestWorldModelRecordParams {
        row_kind: EverQuestWorldModelKind::State,
        row_id: "issue513-state-01".to_owned(),
        profile_id: EVERQUEST_PROFILE_ID.to_owned(),
        payload: serde_json::json!({
            "zone_short_name": "nektulos",
            "coord_bucket": "synthetic-2-plus-2-equals-4",
            "known_expected_value": 4
        }),
        source_refs: vec![EverQuestWorldModelSourceRef {
            kind: "unit_test".to_owned(),
            row_key: Some("synthetic/source".to_owned()),
            path: None,
            start_offset: None,
            next_offset: None,
            content_sha256: None,
            summary: Some("known synthetic source".to_owned()),
        }],
        write_mode: EverQuestWorldModelWriteMode::Create,
        retention_class: EverQuestWorldModelRetentionClass::Strategic,
        compact_redacted: true,
        max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
    }
}
