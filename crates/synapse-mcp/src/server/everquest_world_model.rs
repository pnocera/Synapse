mod model;
#[cfg(test)]
mod tests;
mod validation;

use rmcp::ErrorData;
use synapse_core::error_codes;
use synapse_storage::cf;

use self::model::{
    EverQuestWorldModelInspectParams, EverQuestWorldModelInspectResponse,
    EverQuestWorldModelPrefixCount, EverQuestWorldModelRecordParams,
    EverQuestWorldModelRecordResponse, EverQuestWorldModelRow, INSPECT_SCAN_LIMIT,
    NormalizedInspectParams, NormalizedRecordParams, len_to_u64,
};
use self::validation::{
    build_world_model_row, decode_json_row, inspect_kinds, next_revision, normalize_inspect_params,
    normalize_record_params, sample_rows, selected_row, world_model_prefix,
};
use super::{Json, Parameters, SynapseService, tool, tool_router};
use crate::m1::mcp_error;

const RECORD_TOOL: &str = "everquest_world_model_record";
const INSPECT_TOOL: &str = "everquest_world_model_inspect";

#[tool_router(router = everquest_world_model_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Persist one compact EverQuest world-model row under an approved CF_KV prefix with exact readback"
    )]
    pub async fn everquest_world_model_record(
        &self,
        params: Parameters<EverQuestWorldModelRecordParams>,
    ) -> Result<Json<EverQuestWorldModelRecordResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = RECORD_TOOL,
            "tool.invocation kind=everquest_world_model_record"
        );
        let params = normalize_record_params(params.0)?;
        let response = self.record_world_model_row(params)?;
        Ok(Json(response))
    }

    #[tool(
        description = "Inspect approved EverQuest world-model CF_KV prefixes, selected keys, counts, and redacted samples"
    )]
    pub async fn everquest_world_model_inspect(
        &self,
        params: Parameters<EverQuestWorldModelInspectParams>,
    ) -> Result<Json<EverQuestWorldModelInspectResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = INSPECT_TOOL,
            "tool.invocation kind=everquest_world_model_inspect"
        );
        let params = normalize_inspect_params(params.0)?;
        let response = self.inspect_world_model_rows(params)?;
        Ok(Json(response))
    }
}

impl SynapseService {
    fn record_world_model_row(
        &self,
        params: NormalizedRecordParams,
    ) -> Result<EverQuestWorldModelRecordResponse, ErrorData> {
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "reflex runtime lock poisoned while writing EverQuest world-model row",
            )
        })?;
        let existing = runtime
            .storage_kv_row(params.row_key.as_bytes())
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        let (revision, previous_payload_sha256, created_at, updated_existing) =
            next_revision(&params, existing.as_deref())?;
        let row = build_world_model_row(
            params,
            revision,
            previous_payload_sha256,
            created_at,
            updated_existing,
        );
        let encoded = serde_json::to_vec(&row).map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("encode EverQuest world-model row: {error}"),
            )
        })?;
        runtime
            .storage_put_kv_rows(vec![(row.row_key.as_bytes().to_vec(), encoded)])
            .map_err(|error| {
                mcp_error(
                    error_codes::STORAGE_WRITE_FAILED,
                    format!("write EverQuest world-model row: {error}"),
                )
            })?;
        let stored = runtime
            .storage_kv_row(row.row_key.as_bytes())
            .map_err(|error| {
                mcp_error(
                    error_codes::STORAGE_READ_FAILED,
                    format!("read EverQuest world-model row after write: {error}"),
                )
            })?
            .ok_or_else(|| {
                mcp_error(
                    error_codes::STORAGE_READ_FAILED,
                    format!(
                        "EverQuest world-model row missing after write: {}",
                        row.row_key
                    ),
                )
            })?;
        drop(runtime);
        let readback =
            decode_json_row::<EverQuestWorldModelRow>(&stored, "EverQuest world-model row")?;
        Ok(EverQuestWorldModelRecordResponse {
            ok: true,
            row_key: readback.row_key.clone(),
            stored_value_len_bytes: len_to_u64(stored.len()),
            updated_existing,
            row: readback,
        })
    }

    fn inspect_world_model_rows(
        &self,
        params: NormalizedInspectParams,
    ) -> Result<EverQuestWorldModelInspectResponse, ErrorData> {
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "reflex runtime lock poisoned while inspecting EverQuest world-model rows",
            )
        })?;
        let kinds = inspect_kinds(params.row_kind.as_ref());
        let mut counts = Vec::new();
        let mut samples = Vec::new();
        for kind in kinds {
            let prefix = world_model_prefix(&params.profile_id, &kind);
            let rows = runtime
                .storage_cf_prefix_rows(cf::CF_KV, prefix.as_bytes(), INSPECT_SCAN_LIMIT + 1)
                .map_err(|error| mcp_error(error.code(), error.to_string()))?;
            counts.push(EverQuestWorldModelPrefixCount {
                row_kind: kind.clone(),
                prefix,
                row_count: len_to_u64(rows.len().min(INSPECT_SCAN_LIMIT)),
                scan_truncated: rows.len() > INSPECT_SCAN_LIMIT,
            });
            samples.extend(sample_rows(
                &kind,
                rows.into_iter().take(params.sample_limit),
                params.include_payload,
            )?);
        }
        let selected = params
            .row_key
            .as_deref()
            .map(|key| selected_row(&runtime, key))
            .transpose()?;
        drop(runtime);
        Ok(EverQuestWorldModelInspectResponse {
            ok: true,
            profile_id: params.profile_id,
            cf_name: cf::CF_KV.to_owned(),
            counts,
            samples,
            selected,
        })
    }
}
