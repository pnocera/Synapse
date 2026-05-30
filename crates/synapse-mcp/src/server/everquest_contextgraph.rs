#![allow(clippy::derive_partial_eq_without_eq)]

use std::{path::PathBuf, process::Stdio, time::Duration};

use chrono::{DateTime, Utc};
use rmcp::{ErrorData, schemars::JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use synapse_core::error_codes;
use synapse_storage::cf;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};

use super::{
    Json, Parameters, SynapseService, everquest_episode_export::EverQuestContextGraphEpisodeRow,
    everquest_log::EVERQUEST_PROFILE_ID, tool, tool_router,
};
use crate::m1::mcp_error;

const INGEST_TOOL: &str = "everquest_contextgraph_ingest";
const SEARCH_TOOL: &str = "everquest_contextgraph_search";
const SCHEMA_VERSION: u32 = 1;
const MAX_ID_BYTES: usize = 128;
const MAX_TEXT_BYTES: usize = 512;
const MAX_PATH_BYTES: usize = 1024;
const MAX_QUERY_BYTES: usize = 2048;
const MAX_EPISODES_PER_INGEST: usize = 64;
const CONTEXTGRAPH_CONTENT_LIMIT: usize = 900;
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_IMPORTANCE: f64 = 0.78;
const DEFAULT_TOP_K: u32 = 8;
const MAX_TOP_K: u32 = 25;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestContextGraphIngestParams {
    pub ingest_id: String,
    #[serde(default = "default_profile_id")]
    pub profile_id: String,
    pub export_path: String,
    pub expected_export_sha256: String,
    pub contextgraph_storage_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contextgraph_data_root: Option<String>,
    #[serde(default = "default_contextgraph_command")]
    pub contextgraph_command: String,
    #[serde(default)]
    pub no_warm: bool,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_importance")]
    pub importance: f64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestContextGraphSearchParams {
    pub search_id: String,
    #[serde(default = "default_profile_id")]
    pub profile_id: String,
    pub query: String,
    pub contextgraph_storage_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contextgraph_data_root: Option<String>,
    #[serde(default = "default_contextgraph_command")]
    pub contextgraph_command: String,
    #[serde(default)]
    pub no_warm: bool,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_top_k")]
    pub top_k: u32,
    #[serde(default)]
    pub min_similarity: f64,
    #[serde(default = "default_require_provenance")]
    pub require_provenance: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestContextGraphIngestResponse {
    pub ok: bool,
    pub ingest_id: String,
    pub profile_id: String,
    pub export_path: String,
    pub export_sha256: String,
    pub export_line_count: u32,
    pub contextgraph_command: String,
    pub contextgraph_storage_path: String,
    pub stored_count: u32,
    pub duplicate_count: u32,
    pub rows: Vec<EverQuestContextGraphIngestEpisodeResult>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestContextGraphIngestEpisodeResult {
    pub episode_id: String,
    pub row_key: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint_id: Option<String>,
    pub source_export_sha256: String,
    pub memory_content_sha256: String,
    pub synapse_readback: EverQuestContextGraphBridgeReadback,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contextgraph_store_readback: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contextgraph_provenance_readback: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contextgraph_audit_readback: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestContextGraphSearchResponse {
    pub ok: bool,
    pub search_id: String,
    pub profile_id: String,
    pub query: String,
    pub contextgraph_command: String,
    pub contextgraph_storage_path: String,
    pub result_count: u32,
    pub citation_count: u32,
    pub citations: Vec<EverQuestContextGraphCitation>,
    pub contextgraph_search_readback: Value,
    pub synapse_readback: EverQuestContextGraphBridgeReadback,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestContextGraphCitation {
    pub fingerprint_id: String,
    pub source_episode_id: String,
    pub source_export_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestContextGraphBridgeReadback {
    pub cf_name: String,
    pub row_key: String,
    pub found: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_len_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_kind: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct EverQuestContextGraphIngestRow {
    schema_version: u32,
    row_kind: String,
    ingest_id: String,
    profile_id: String,
    episode_id: String,
    row_key: String,
    created_at: DateTime<Utc>,
    source_export_path: String,
    source_export_sha256: String,
    source_episode_content_sha256: String,
    contextgraph_command: String,
    contextgraph_storage_path: String,
    fingerprint_id: String,
    tags: Vec<String>,
    memory_content_sha256: String,
    memory_content_len_bytes: u64,
    contextgraph_store_readback: Value,
    contextgraph_provenance_readback: Value,
    contextgraph_audit_readback: Value,
    redaction: EverQuestContextGraphRedaction,
    evidence_boundary: EverQuestContextGraphEvidenceBoundary,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct EverQuestContextGraphSearchRow {
    schema_version: u32,
    row_kind: String,
    search_id: String,
    profile_id: String,
    row_key: String,
    created_at: DateTime<Utc>,
    query: String,
    contextgraph_command: String,
    contextgraph_storage_path: String,
    result_count: u32,
    citation_count: u32,
    citations: Vec<EverQuestContextGraphCitation>,
    contextgraph_search_readback: Value,
    evidence_boundary: EverQuestContextGraphEvidenceBoundary,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct EverQuestContextGraphRedaction {
    compact_redacted: bool,
    raw_chat_body_persisted: bool,
    raw_target_names_persisted: bool,
    raw_session_id_persisted: bool,
    private_session_id_hash_only: bool,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct EverQuestContextGraphEvidenceBoundary {
    synapse_storage_is_gameplay_sot: bool,
    contextgraph_is_long_term_retrieval: bool,
    manual_fsv_required_for_gameplay_claims: bool,
    is_fsv_script: bool,
    note: String,
}

#[derive(Clone, Debug)]
struct NormalizedIngestParams {
    ingest_id: String,
    profile_id: String,
    export_path: PathBuf,
    expected_export_sha256: String,
    contextgraph: ContextGraphLaunchConfig,
    importance: f64,
}

#[derive(Clone, Debug)]
struct NormalizedSearchParams {
    search_id: String,
    profile_id: String,
    query: String,
    contextgraph: ContextGraphLaunchConfig,
    top_k: u32,
    min_similarity: f64,
    require_provenance: bool,
}

#[derive(Clone, Debug)]
struct ContextGraphLaunchConfig {
    command: String,
    storage_path: PathBuf,
    data_root: Option<PathBuf>,
    no_warm: bool,
    timeout: Duration,
}

#[derive(Clone, Debug)]
struct EpisodeExportReadback {
    path: PathBuf,
    sha256: String,
    rows: Vec<EverQuestContextGraphEpisodeRow>,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug)]
struct ContextGraphTools {
    store_memory: bool,
    search_graph: bool,
    get_provenance_chain: bool,
    get_audit_trail: bool,
}

struct ContextGraphClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    timeout: Duration,
}

#[tool_router(router = everquest_contextgraph_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Ingest redacted EverQuest episode JSONL into ContextGraph through its MCP stdio tool surface and persist Synapse bridge readbacks"
    )]
    pub async fn everquest_contextgraph_ingest(
        &self,
        params: Parameters<EverQuestContextGraphIngestParams>,
    ) -> Result<Json<EverQuestContextGraphIngestResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = INGEST_TOOL,
            "tool.invocation kind=everquest_contextgraph_ingest"
        );
        let params = normalize_ingest_params(&params.0)?;
        let response = self.ingest_contextgraph_episodes(params).await?;
        Ok(Json(response))
    }

    #[tool(
        description = "Query ContextGraph for EverQuest memories, require source episode/hash provenance, and persist a Synapse search audit row"
    )]
    pub async fn everquest_contextgraph_search(
        &self,
        params: Parameters<EverQuestContextGraphSearchParams>,
    ) -> Result<Json<EverQuestContextGraphSearchResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = SEARCH_TOOL,
            "tool.invocation kind=everquest_contextgraph_search"
        );
        let params = normalize_search_params(&params.0)?;
        let response = self.search_contextgraph_episodes(params).await?;
        Ok(Json(response))
    }
}

impl SynapseService {
    #[allow(clippy::too_many_lines)]
    async fn ingest_contextgraph_episodes(
        &self,
        params: NormalizedIngestParams,
    ) -> Result<EverQuestContextGraphIngestResponse, ErrorData> {
        let export = read_episode_export(&params)?;
        let mut client: Option<ContextGraphClient> = None;

        let mut rows = Vec::new();
        let mut stored_count = 0_usize;
        let mut duplicate_count = 0_usize;
        for episode in &export.rows {
            let memory_content = build_memory_content(episode, &export, &params)?;
            let memory_content_sha256 = sha256_hex(memory_content.as_bytes());
            let row_key = ingest_row_key(&params.profile_id, &export.sha256, &episode.episode_id);
            if let Some(existing) = self.read_ingest_row(&row_key)? {
                if existing.source_export_sha256 != export.sha256 {
                    return Err(mcp_error(
                        error_codes::STORAGE_CORRUPTED,
                        format!("ContextGraph ingest row hash mismatch: {row_key}"),
                    ));
                }
                duplicate_count = duplicate_count.saturating_add(1);
                let readback = self.read_bridge_row(&row_key)?;
                rows.push(EverQuestContextGraphIngestEpisodeResult {
                    episode_id: episode.episode_id.clone(),
                    row_key,
                    status: "duplicate_existing_readback".to_owned(),
                    fingerprint_id: Some(existing.fingerprint_id),
                    source_export_sha256: existing.source_export_sha256,
                    memory_content_sha256,
                    synapse_readback: readback,
                    contextgraph_store_readback: Some(existing.contextgraph_store_readback),
                    contextgraph_provenance_readback: Some(
                        existing.contextgraph_provenance_readback,
                    ),
                    contextgraph_audit_readback: Some(existing.contextgraph_audit_readback),
                });
                continue;
            }

            if client.is_none() {
                let mut launched = ContextGraphClient::launch(&params.contextgraph)?;
                let tools = launched.initialize_and_list_tools().await?;
                tools.require_ingest_tools()?;
                client = Some(launched);
            }
            let client = client.as_mut().ok_or_else(|| {
                mcp_error(
                    error_codes::MODEL_BACKEND_UNAVAILABLE,
                    "ContextGraph MCP client was not initialized for new ingest row",
                )
            })?;

            let store_readback = client
                .tool_call(
                    "store_memory",
                    json!({
                        "content": memory_content,
                        "rationale": "Synapse compact EverQuest episode exported from verified local storage for long-term world memory retrieval.",
                        "importance": params.importance,
                        "sessionId": format!("synapse-everquest-{}", params.ingest_id),
                        "operatorId": "synapse-mcp"
                    }),
                )
                .await?;
            let fingerprint_id = required_string(&store_readback, "fingerprintId")?;
            let provenance_readback = client
                .tool_call(
                    "get_provenance_chain",
                    json!({
                        "memory_id": fingerprint_id,
                        "include_audit": true,
                        "include_embedding_version": true
                    }),
                )
                .await?;
            let audit_readback = client
                .tool_call(
                    "get_audit_trail",
                    json!({
                        "target_id": fingerprint_id,
                        "limit": 20
                    }),
                )
                .await?;
            let ingest_row = EverQuestContextGraphIngestRow {
                schema_version: SCHEMA_VERSION,
                row_kind: "everquest_contextgraph_ingest".to_owned(),
                ingest_id: params.ingest_id.clone(),
                profile_id: params.profile_id.clone(),
                episode_id: episode.episode_id.clone(),
                row_key: row_key.clone(),
                created_at: Utc::now(),
                source_export_path: export.path.display().to_string(),
                source_export_sha256: export.sha256.clone(),
                source_episode_content_sha256: sha256_hex(
                    serde_json::to_string(episode)
                        .map_err(|error| {
                            mcp_error(
                                error_codes::TOOL_INTERNAL_ERROR,
                                format!("encode episode for hash: {error}"),
                            )
                        })?
                        .as_bytes(),
                ),
                contextgraph_command: params.contextgraph.command.clone(),
                contextgraph_storage_path: params.contextgraph.storage_path.display().to_string(),
                fingerprint_id: fingerprint_id.clone(),
                tags: everquest_tags(&params.profile_id),
                memory_content_sha256: memory_content_sha256.clone(),
                memory_content_len_bytes: len_to_u64(memory_content.len()),
                contextgraph_store_readback: store_readback.clone(),
                contextgraph_provenance_readback: provenance_readback.clone(),
                contextgraph_audit_readback: audit_readback.clone(),
                redaction: bridge_redaction(),
                evidence_boundary: evidence_boundary(),
            };
            let synapse_readback = self.write_ingest_row(&ingest_row)?;
            stored_count = stored_count.saturating_add(1);
            rows.push(EverQuestContextGraphIngestEpisodeResult {
                episode_id: episode.episode_id.clone(),
                row_key,
                status: "stored".to_owned(),
                fingerprint_id: Some(fingerprint_id),
                source_export_sha256: export.sha256.clone(),
                memory_content_sha256,
                synapse_readback,
                contextgraph_store_readback: Some(store_readback),
                contextgraph_provenance_readback: Some(provenance_readback),
                contextgraph_audit_readback: Some(audit_readback),
            });
        }
        if let Some(client) = client {
            client.shutdown().await;
        }

        Ok(EverQuestContextGraphIngestResponse {
            ok: true,
            ingest_id: params.ingest_id,
            profile_id: params.profile_id,
            export_path: export.path.display().to_string(),
            export_sha256: export.sha256,
            export_line_count: len_to_u32(export.rows.len()),
            contextgraph_command: params.contextgraph.command,
            contextgraph_storage_path: params.contextgraph.storage_path.display().to_string(),
            stored_count: len_to_u32(stored_count),
            duplicate_count: len_to_u32(duplicate_count),
            rows,
        })
    }

    async fn search_contextgraph_episodes(
        &self,
        params: NormalizedSearchParams,
    ) -> Result<EverQuestContextGraphSearchResponse, ErrorData> {
        let mut client = ContextGraphClient::launch(&params.contextgraph)?;
        let tools = client.initialize_and_list_tools().await?;
        tools.require_search_tools()?;
        let tagged_query = format!(
            "{} game:everquest character:Thenumberone server:frostreaver zone:neriaka",
            params.query
        );
        let search_readback = client
            .tool_call(
                "search_graph",
                json!({
                    "query": tagged_query,
                    "topK": params.top_k,
                    "minSimilarity": params.min_similarity,
                    "includeContent": true,
                    "includeProvenance": true,
                    "strategy": "pipeline",
                    "sessionId": format!("synapse-everquest-search-{}", params.search_id)
                }),
            )
            .await?;
        client.shutdown().await;

        let citations = extract_citations(&search_readback);
        if params.require_provenance && citations.is_empty() {
            return Err(mcp_error(
                error_codes::STORAGE_READ_FAILED,
                "ContextGraph search returned no EverQuest source episode/hash provenance",
            ));
        }
        let result_count = search_readback
            .get("results")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let row_key = search_row_key(&params.profile_id, &params.search_id);
        let search_row = EverQuestContextGraphSearchRow {
            schema_version: SCHEMA_VERSION,
            row_kind: "everquest_contextgraph_search".to_owned(),
            search_id: params.search_id.clone(),
            profile_id: params.profile_id.clone(),
            row_key,
            created_at: Utc::now(),
            query: params.query.clone(),
            contextgraph_command: params.contextgraph.command.clone(),
            contextgraph_storage_path: params.contextgraph.storage_path.display().to_string(),
            result_count: len_to_u32(result_count),
            citation_count: len_to_u32(citations.len()),
            citations: citations.clone(),
            contextgraph_search_readback: search_readback.clone(),
            evidence_boundary: evidence_boundary(),
        };
        let synapse_readback = self.write_search_row(&search_row)?;
        Ok(EverQuestContextGraphSearchResponse {
            ok: true,
            search_id: params.search_id,
            profile_id: params.profile_id,
            query: params.query,
            contextgraph_command: params.contextgraph.command,
            contextgraph_storage_path: params.contextgraph.storage_path.display().to_string(),
            result_count: len_to_u32(result_count),
            citation_count: len_to_u32(citations.len()),
            citations,
            contextgraph_search_readback: search_readback,
            synapse_readback,
        })
    }

    fn read_ingest_row(
        &self,
        row_key: &str,
    ) -> Result<Option<EverQuestContextGraphIngestRow>, ErrorData> {
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "reflex runtime lock poisoned while reading ContextGraph ingest row",
            )
        })?;
        let existing = runtime
            .storage_kv_row(row_key.as_bytes())
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        drop(runtime);
        existing
            .map(|bytes| decode_json_row::<EverQuestContextGraphIngestRow>(&bytes, row_key))
            .transpose()
    }

    fn write_ingest_row(
        &self,
        row: &EverQuestContextGraphIngestRow,
    ) -> Result<EverQuestContextGraphBridgeReadback, ErrorData> {
        self.write_bridge_row(&row.row_key, row, "ContextGraph ingest bridge row")
    }

    fn write_search_row(
        &self,
        row: &EverQuestContextGraphSearchRow,
    ) -> Result<EverQuestContextGraphBridgeReadback, ErrorData> {
        self.write_bridge_row(&row.row_key, row, "ContextGraph search bridge row")
    }

    fn write_bridge_row<T>(
        &self,
        row_key: &str,
        row: &T,
        label: &str,
    ) -> Result<EverQuestContextGraphBridgeReadback, ErrorData>
    where
        T: Serialize,
    {
        let encoded = serde_json::to_vec(row).map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("encode {label}: {error}"),
            )
        })?;
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("reflex runtime lock poisoned while writing {label}"),
            )
        })?;
        runtime
            .storage_put_kv_rows(vec![(row_key.as_bytes().to_vec(), encoded)])
            .map_err(|error| {
                mcp_error(
                    error_codes::STORAGE_WRITE_FAILED,
                    format!("write {label}: {error}"),
                )
            })?;
        let stored = runtime
            .storage_kv_row(row_key.as_bytes())
            .map_err(|error| {
                mcp_error(
                    error_codes::STORAGE_READ_FAILED,
                    format!("read {label} after write: {error}"),
                )
            })?;
        drop(runtime);
        bridge_readback(row_key, stored.as_deref())
    }

    fn read_bridge_row(
        &self,
        row_key: &str,
    ) -> Result<EverQuestContextGraphBridgeReadback, ErrorData> {
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "reflex runtime lock poisoned while reading ContextGraph bridge row",
            )
        })?;
        let stored = runtime
            .storage_kv_row(row_key.as_bytes())
            .map_err(|error| mcp_error(error.code(), error.to_string()))?;
        drop(runtime);
        bridge_readback(row_key, stored.as_deref())
    }
}

impl ContextGraphClient {
    fn launch(config: &ContextGraphLaunchConfig) -> Result<Self, ErrorData> {
        let mut command = Command::new(&config.command);
        command
            .arg("--transport")
            .arg("stdio")
            .env("CONTEXT_GRAPH_STORAGE_PATH", &config.storage_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if let Some(data_root) = &config.data_root {
            command.env("CONTEXTGRAPH_DATA_ROOT", data_root);
        }
        if config.no_warm {
            command.arg("--no-warm");
        }
        let mut child = command.spawn().map_err(|error| {
            mcp_error(
                error_codes::MODEL_BACKEND_UNAVAILABLE,
                format!(
                    "launch ContextGraph MCP command {:?} failed: {error}",
                    config.command
                ),
            )
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            mcp_error(
                error_codes::MODEL_BACKEND_UNAVAILABLE,
                "ContextGraph MCP stdin was not available",
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            mcp_error(
                error_codes::MODEL_BACKEND_UNAVAILABLE,
                "ContextGraph MCP stdout was not available",
            )
        })?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            timeout: config.timeout,
        })
    }

    async fn initialize_and_list_tools(&mut self) -> Result<ContextGraphTools, ErrorData> {
        let _initialize = self.send_request("initialize", json!({})).await?;
        let tools_result = self.send_request("tools/list", json!({})).await?;
        let tools = tools_result
            .get("result")
            .and_then(|result| result.get("tools"))
            .and_then(Value::as_array)
            .ok_or_else(|| {
                mcp_error(
                    error_codes::MODEL_BACKEND_UNAVAILABLE,
                    "ContextGraph MCP tools/list response did not contain tools array",
                )
            })?;
        Ok(ContextGraphTools {
            store_memory: tool_present(tools, "store_memory"),
            search_graph: tool_present(tools, "search_graph"),
            get_provenance_chain: tool_present(tools, "get_provenance_chain"),
            get_audit_trail: tool_present(tools, "get_audit_trail"),
        })
    }

    async fn tool_call(&mut self, name: &str, arguments: Value) -> Result<Value, ErrorData> {
        let response = self
            .send_request(
                "tools/call",
                json!({
                    "name": name,
                    "arguments": arguments
                }),
            )
            .await?;
        let result = response.get("result").ok_or_else(|| {
            mcp_error(
                error_codes::MODEL_BACKEND_UNAVAILABLE,
                format!("ContextGraph tool {name} response missing result"),
            )
        })?;
        if result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Err(mcp_error(
                error_codes::MODEL_BACKEND_UNAVAILABLE,
                format!(
                    "ContextGraph tool {name} failed closed: {}",
                    compact_json(result)
                ),
            ));
        }
        if let Some(structured) = result.get("structuredContent") {
            return Ok(structured.clone());
        }
        parse_tool_content_json(result).ok_or_else(|| {
            mcp_error(
                error_codes::MODEL_BACKEND_UNAVAILABLE,
                format!("ContextGraph tool {name} response missing structured content"),
            )
        })
    }

    async fn send_request(&mut self, method: &str, params: Value) -> Result<Value, ErrorData> {
        let request_id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let request = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params
        });
        let request_line = serde_json::to_string(&request).map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("encode ContextGraph JSON-RPC request: {error}"),
            )
        })?;
        self.stdin
            .write_all(request_line.as_bytes())
            .await
            .map_err(|error| {
                mcp_error(
                    error_codes::MODEL_BACKEND_UNAVAILABLE,
                    format!("write ContextGraph JSON-RPC request: {error}"),
                )
            })?;
        self.stdin.write_all(b"\n").await.map_err(|error| {
            mcp_error(
                error_codes::MODEL_BACKEND_UNAVAILABLE,
                format!("write ContextGraph JSON-RPC newline: {error}"),
            )
        })?;
        self.stdin.flush().await.map_err(|error| {
            mcp_error(
                error_codes::MODEL_BACKEND_UNAVAILABLE,
                format!("flush ContextGraph JSON-RPC request: {error}"),
            )
        })?;
        for _ in 0..16 {
            let mut line = String::new();
            let read = timeout(self.timeout, self.stdout.read_line(&mut line))
                .await
                .map_err(|_| {
                    mcp_error(
                        error_codes::MODEL_BACKEND_UNAVAILABLE,
                        format!("ContextGraph JSON-RPC request timed out: {method}"),
                    )
                })?
                .map_err(|error| {
                    mcp_error(
                        error_codes::MODEL_BACKEND_UNAVAILABLE,
                        format!("read ContextGraph JSON-RPC response: {error}"),
                    )
                })?;
            if read == 0 {
                return Err(mcp_error(
                    error_codes::MODEL_BACKEND_UNAVAILABLE,
                    format!("ContextGraph MCP exited before responding to {method}"),
                ));
            }
            let response = serde_json::from_str::<Value>(line.trim()).map_err(|error| {
                mcp_error(
                    error_codes::MODEL_BACKEND_UNAVAILABLE,
                    format!("decode ContextGraph JSON-RPC response: {error}"),
                )
            })?;
            if response.get("id").and_then(Value::as_u64) != Some(request_id) {
                continue;
            }
            if let Some(error) = response.get("error") {
                return Err(mcp_error(
                    error_codes::MODEL_BACKEND_UNAVAILABLE,
                    format!("ContextGraph JSON-RPC error for {method}: {error}"),
                ));
            }
            return Ok(response);
        }
        Err(mcp_error(
            error_codes::MODEL_BACKEND_UNAVAILABLE,
            format!("ContextGraph JSON-RPC response id not found for {method}"),
        ))
    }

    async fn shutdown(mut self) {
        let _ = self.send_request("shutdown", json!({})).await;
        if timeout(Duration::from_secs(5), self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
        }
    }
}

impl ContextGraphTools {
    fn require_ingest_tools(&self) -> Result<(), ErrorData> {
        if self.store_memory && self.get_provenance_chain && self.get_audit_trail {
            return Ok(());
        }
        Err(mcp_error(
            error_codes::MODEL_BACKEND_UNAVAILABLE,
            format!(
                "ContextGraph MCP missing required ingest tools: store_memory={}, get_provenance_chain={}, get_audit_trail={}",
                self.store_memory, self.get_provenance_chain, self.get_audit_trail
            ),
        ))
    }

    fn require_search_tools(&self) -> Result<(), ErrorData> {
        if self.search_graph {
            return Ok(());
        }
        Err(mcp_error(
            error_codes::MODEL_BACKEND_UNAVAILABLE,
            format!(
                "ContextGraph MCP missing required search tool: search_graph={}",
                self.search_graph
            ),
        ))
    }
}

fn normalize_ingest_params(
    params: &EverQuestContextGraphIngestParams,
) -> Result<NormalizedIngestParams, ErrorData> {
    let ingest_id = validate_id("ingest_id", &params.ingest_id)?;
    let profile_id = validate_profile_id(&params.profile_id)?;
    let export_path = normalize_file_path("export_path", &params.export_path, Some("jsonl"))?;
    let expected_export_sha256 =
        validate_sha256("expected_export_sha256", &params.expected_export_sha256)?;
    let contextgraph = normalize_contextgraph_config(
        &params.contextgraph_command,
        &params.contextgraph_storage_path,
        params.contextgraph_data_root.as_deref(),
        params.no_warm,
        params.timeout_ms,
    )?;
    if !(0.0..=1.0).contains(&params.importance) {
        return Err(params_error("importance must be 0.0..=1.0"));
    }
    Ok(NormalizedIngestParams {
        ingest_id,
        profile_id,
        export_path,
        expected_export_sha256,
        contextgraph,
        importance: params.importance,
    })
}

fn normalize_search_params(
    params: &EverQuestContextGraphSearchParams,
) -> Result<NormalizedSearchParams, ErrorData> {
    let search_id = validate_id("search_id", &params.search_id)?;
    let profile_id = validate_profile_id(&params.profile_id)?;
    let query = normalize_query(&params.query)?;
    let contextgraph = normalize_contextgraph_config(
        &params.contextgraph_command,
        &params.contextgraph_storage_path,
        params.contextgraph_data_root.as_deref(),
        params.no_warm,
        params.timeout_ms,
    )?;
    if params.top_k == 0 || params.top_k > MAX_TOP_K {
        return Err(params_error(format!("top_k must be 1..={MAX_TOP_K}")));
    }
    if !(0.0..=1.0).contains(&params.min_similarity) {
        return Err(params_error("min_similarity must be 0.0..=1.0"));
    }
    Ok(NormalizedSearchParams {
        search_id,
        profile_id,
        query,
        contextgraph,
        top_k: params.top_k,
        min_similarity: params.min_similarity,
        require_provenance: params.require_provenance,
    })
}

fn normalize_contextgraph_config(
    command: &str,
    storage_path: &str,
    data_root: Option<&str>,
    no_warm: bool,
    timeout_ms: u64,
) -> Result<ContextGraphLaunchConfig, ErrorData> {
    let command = normalize_required_text("contextgraph_command", command)?;
    let storage_path = normalize_dir_path("contextgraph_storage_path", storage_path)?;
    let data_root = data_root
        .map(|value| normalize_dir_path("contextgraph_data_root", value))
        .transpose()?;
    if !(1_000..=600_000).contains(&timeout_ms) {
        return Err(params_error("timeout_ms must be 1000..=600000"));
    }
    Ok(ContextGraphLaunchConfig {
        command,
        storage_path,
        data_root,
        no_warm,
        timeout: Duration::from_millis(timeout_ms),
    })
}

fn read_episode_export(
    params: &NormalizedIngestParams,
) -> Result<EpisodeExportReadback, ErrorData> {
    let bytes = std::fs::read(&params.export_path).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_READ_FAILED,
            format!(
                "read EverQuest ContextGraph episode export {}: {error}",
                params.export_path.display()
            ),
        )
    })?;
    let actual_sha256 = sha256_hex(&bytes);
    if actual_sha256 != params.expected_export_sha256 {
        return Err(mcp_error(
            error_codes::STORAGE_SCHEMA_MISMATCH,
            format!(
                "episode export hash mismatch: expected {}, got {actual_sha256}",
                params.expected_export_sha256
            ),
        ));
    }
    let mut rows = Vec::new();
    for (index, line) in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .enumerate()
    {
        if rows.len() >= MAX_EPISODES_PER_INGEST {
            return Err(params_error(format!(
                "episode export must contain <= {MAX_EPISODES_PER_INGEST} rows per ingest"
            )));
        }
        let row =
            serde_json::from_slice::<EverQuestContextGraphEpisodeRow>(line).map_err(|error| {
                mcp_error(
                    error_codes::STORAGE_CORRUPTED,
                    format!("decode EverQuest episode JSONL line {}: {error}", index + 1),
                )
            })?;
        validate_episode_row(index, &row, &params.profile_id)?;
        rows.push(row);
    }
    if rows.is_empty() {
        return Err(params_error("episode export must not be empty"));
    }
    Ok(EpisodeExportReadback {
        path: params.export_path.clone(),
        sha256: actual_sha256,
        rows,
    })
}

fn validate_episode_row(
    index: usize,
    row: &EverQuestContextGraphEpisodeRow,
    profile_id: &str,
) -> Result<(), ErrorData> {
    let line = index.saturating_add(1);
    if row.schema_version != SCHEMA_VERSION {
        return Err(mcp_error(
            error_codes::STORAGE_SCHEMA_MISMATCH,
            format!("episode line {line} schema_version must be {SCHEMA_VERSION}"),
        ));
    }
    if row.record_kind != "everquest_contextgraph_dynamicjepa_episode" {
        return Err(mcp_error(
            error_codes::STORAGE_SCHEMA_MISMATCH,
            format!("episode line {line} record_kind is not exportable"),
        ));
    }
    if row.profile_id != profile_id {
        return Err(params_error(format!(
            "episode line {line} profile_id mismatch: expected {profile_id}, got {}",
            row.profile_id
        )));
    }
    if row.contextgraph.get("compatible").and_then(Value::as_bool) != Some(true)
        || row.contextgraph.get("format").and_then(Value::as_str)
            != Some("dynamicjepa_episode_jsonl")
    {
        return Err(mcp_error(
            error_codes::STORAGE_SCHEMA_MISMATCH,
            format!("episode line {line} ContextGraph compatibility block is invalid"),
        ));
    }
    if !row.redaction.compact_redacted
        || row.redaction.raw_chat_body_persisted
        || row.redaction.raw_target_names_persisted
        || row.redaction.raw_session_id_persisted
        || !row.redaction.private_session_id_hash_only
        || !row.redaction.all_log_refs_marked_redacted
    {
        return Err(params_error(format!(
            "episode line {line} redaction block is not safe for ContextGraph ingest"
        )));
    }
    let value = serde_json::to_value(row).map_err(|error| {
        mcp_error(
            error_codes::TOOL_INTERNAL_ERROR,
            format!("encode episode line {line} for validation: {error}"),
        )
    })?;
    if contains_forbidden_raw_payload(&value) {
        return Err(params_error(format!(
            "episode line {line} appears to contain private chat/session/target payloads"
        )));
    }
    Ok(())
}

fn build_memory_content(
    episode: &EverQuestContextGraphEpisodeRow,
    export: &EpisodeExportReadback,
    params: &NormalizedIngestParams,
) -> Result<String, ErrorData> {
    let mut content = format!(
        "Synapse EverQuest memory. Tags: {}. Profile: {}.\n\
         source_episode_id={}\n\
         source_export_sha256={}\n\
         Character {} on {}. State: zone {} ({}), level {}, coord {}, hp {}, mana {}, focus {}, map {}, target {}.\n\
         Action: {} via {} from {}. Outcome: {}, log {}, next zone {}, next coord {}, xp {}, damage {}, death {}, surprise {}, ui {}.\n\
         Planning: accepted {}, invariants {}, candidates {}. Evidence: compact redacted #521 export only; no raw chat, raw target names, or raw session id. Gameplay claims still require attended physical FSV.\n",
        everquest_tags(&params.profile_id).join(" "),
        params.profile_id,
        episode.episode_id,
        export.sha256,
        value_string(&episode.transition, &["entity", "character_summary"]),
        value_string(&episode.transition, &["entity", "server"]),
        fields_string(&episode.state, "zone_short_name"),
        fields_string(&episode.state, "zone_display_name"),
        fields_string(&episode.state, "level_bucket"),
        fields_string(&episode.state, "coord_bucket"),
        fields_string(&episode.state, "hp_bucket"),
        fields_string(&episode.state, "mana_bucket"),
        fields_string(&episode.state, "ui_focus_bucket"),
        fields_string(&episode.state, "map_visible"),
        fields_string(&episode.state, "target_kind"),
        fields_string(&episode.action, "action_kind"),
        fields_string(&episode.action, "tool_name"),
        fields_string(&episode.action, "action_origin"),
        fields_string(&episode.outcome, "outcome_kind"),
        fields_string(&episode.outcome, "log_event_kind"),
        fields_string(&episode.outcome, "next_zone_short_name"),
        fields_string(&episode.outcome, "next_coord_bucket"),
        fields_string(&episode.outcome, "xp_delta"),
        fields_string(&episode.outcome, "damage_delta"),
        fields_string(&episode.outcome, "death_delta"),
        fields_string(&episode.outcome, "surprise"),
        fields_string(&episode.outcome, "ui_mutation"),
        value_string(&episode.transition, &["accepted_for_planning"]),
        invariant_summary(&episode.transition),
        candidate_actions_summary(&episode.transition)
    );
    if content.len() > CONTEXTGRAPH_CONTENT_LIMIT {
        content.truncate(CONTEXTGRAPH_CONTENT_LIMIT);
    }
    if contains_forbidden_raw_payload(&Value::String(content.clone())) {
        return Err(params_error(
            "generated ContextGraph memory content appears to contain private payloads",
        ));
    }
    Ok(content)
}

fn fields_string(value: &Value, key: &str) -> String {
    value_string(value, &["fields", key])
}

fn value_string(value: &Value, path: &[&str]) -> String {
    let mut current = value;
    for key in path {
        let Some(next) = current.get(*key) else {
            return "unknown".to_owned();
        };
        current = next;
    }
    match current {
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Null => "null".to_owned(),
        _ => compact_json(current),
    }
}

fn invariant_summary(transition: &Value) -> String {
    let Some(invariants) = transition
        .get("invariant_results")
        .and_then(Value::as_array)
    else {
        return "unknown".to_owned();
    };
    let total = invariants.len();
    let passed = invariants
        .iter()
        .filter(|item| item.get("passed").and_then(Value::as_bool) == Some(true))
        .count();
    let fatal_failed = invariants
        .iter()
        .filter(|item| {
            item.get("severity").and_then(Value::as_str) == Some("fatal")
                && item.get("passed").and_then(Value::as_bool) != Some(true)
        })
        .count();
    format!("passed_{passed}_of_{total}_fatal_failed_{fatal_failed}")
}

fn candidate_actions_summary(transition: &Value) -> String {
    let Some(actions) = transition
        .get("planner_policy")
        .and_then(|policy| policy.get("candidate_actions"))
        .and_then(Value::as_array)
    else {
        return "unknown".to_owned();
    };
    actions
        .iter()
        .filter_map(Value::as_str)
        .take(10)
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_tool_content_json(result: &Value) -> Option<Value> {
    let text = result
        .get("content")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|item| item.get("text").and_then(Value::as_str))?;
    serde_json::from_str(text).ok()
}

fn tool_present(tools: &[Value], name: &str) -> bool {
    tools
        .iter()
        .any(|tool| tool.get("name").and_then(Value::as_str) == Some(name))
}

fn required_string(value: &Value, key: &str) -> Result<String, ErrorData> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            mcp_error(
                error_codes::MODEL_BACKEND_UNAVAILABLE,
                format!("ContextGraph response missing string field {key}"),
            )
        })
}

fn bridge_readback(
    row_key: &str,
    stored: Option<&[u8]>,
) -> Result<EverQuestContextGraphBridgeReadback, ErrorData> {
    let Some(stored) = stored else {
        return Ok(EverQuestContextGraphBridgeReadback {
            cf_name: cf::CF_KV.to_owned(),
            row_key: row_key.to_owned(),
            found: false,
            value_len_bytes: None,
            sha256: None,
            row_kind: None,
        });
    };
    let value = serde_json::from_slice::<Value>(stored).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_CORRUPTED,
            format!("decode ContextGraph bridge row {row_key}: {error}"),
        )
    })?;
    Ok(EverQuestContextGraphBridgeReadback {
        cf_name: cf::CF_KV.to_owned(),
        row_key: row_key.to_owned(),
        found: true,
        value_len_bytes: Some(len_to_u64(stored.len())),
        sha256: Some(sha256_hex(stored)),
        row_kind: value
            .get("row_kind")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn extract_citations(search_readback: &Value) -> Vec<EverQuestContextGraphCitation> {
    let Some(results) = search_readback.get("results").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut citations = Vec::new();
    for result in results {
        let Some(fingerprint_id) = result
            .get("fingerprintId")
            .or_else(|| result.get("fingerprint_id"))
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
        else {
            continue;
        };
        let mut strings = Vec::new();
        collect_strings(result, &mut strings);
        for text in strings {
            if let (Some(source_episode_id), Some(source_export_sha256)) = (
                marker_value(text, "source_episode_id="),
                marker_value(text, "source_export_sha256="),
            ) && valid_source_episode_id(&source_episode_id)
                && is_sha256_hex(&source_export_sha256)
            {
                citations.push(EverQuestContextGraphCitation {
                    fingerprint_id: fingerprint_id.clone(),
                    source_episode_id,
                    source_export_sha256: source_export_sha256.to_ascii_lowercase(),
                });
            }
        }
    }
    citations.sort_by(|left, right| {
        left.fingerprint_id
            .cmp(&right.fingerprint_id)
            .then(left.source_episode_id.cmp(&right.source_episode_id))
    });
    citations.dedup();
    citations
}

fn collect_strings<'a>(value: &'a Value, output: &mut Vec<&'a str>) {
    match value {
        Value::String(text) => output.push(text),
        Value::Array(values) => {
            for item in values {
                collect_strings(item, output);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                collect_strings(item, output);
            }
        }
        _ => {}
    }
}

fn marker_value(text: &str, marker: &str) -> Option<String> {
    let start = text.find(marker)?.saturating_add(marker.len());
    let tail = &text[start..];
    let end = tail
        .find(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ';' | ')' | ']'))
        .unwrap_or(tail.len());
    let value = tail[..end].trim_matches(|ch| matches!(ch, '"' | '\''));
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

fn valid_source_episode_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit())
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
    matches!(
        key.as_str(),
        "raw_chat"
            | "raw_chat_body"
            | "chat_body"
            | "chat_text"
            | "message_text"
            | "say_text"
            | "tell_text"
            | "raw_target_name"
            | "raw_target_names"
            | "target_name"
            | "target_names"
            | "raw_session"
            | "raw_session_id"
            | "session_id"
    )
}

fn forbidden_raw_string(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.contains("you say,")
        || value.contains("you tell ")
        || value.contains("tells you,")
        || value.contains("/say ")
        || value.contains("/tell ")
        || value.contains("raw player chat")
}

fn everquest_tags(profile_id: &str) -> Vec<String> {
    vec![
        "game:everquest".to_owned(),
        "character:Thenumberone".to_owned(),
        "server:frostreaver".to_owned(),
        "zone:neriaka".to_owned(),
        format!("profile:{profile_id}"),
    ]
}

const fn bridge_redaction() -> EverQuestContextGraphRedaction {
    EverQuestContextGraphRedaction {
        compact_redacted: true,
        raw_chat_body_persisted: false,
        raw_target_names_persisted: false,
        raw_session_id_persisted: false,
        private_session_id_hash_only: true,
    }
}

fn evidence_boundary() -> EverQuestContextGraphEvidenceBoundary {
    EverQuestContextGraphEvidenceBoundary {
        synapse_storage_is_gameplay_sot: true,
        contextgraph_is_long_term_retrieval: true,
        manual_fsv_required_for_gameplay_claims: true,
        is_fsv_script: false,
        note: "ContextGraph retrieval is durable memory/provenance only; Synapse storage, EQ logs, and UI remain the gameplay FSV sources of truth."
            .to_owned(),
    }
}

fn ingest_row_key(profile_id: &str, export_sha256: &str, episode_id: &str) -> String {
    format!("everquest/contextgraph_ingest/v1/{profile_id}/{export_sha256}/{episode_id}")
}

fn search_row_key(profile_id: &str, search_id: &str) -> String {
    format!("everquest/contextgraph_search/v1/{profile_id}/{search_id}")
}

fn normalize_file_path(
    field: &str,
    value: &str,
    required_extension: Option<&str>,
) -> Result<PathBuf, ErrorData> {
    let value = normalize_path_text(field, value)?;
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(params_error(format!("{field} must be an absolute path")));
    }
    if let Some(extension) = required_extension
        && !path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case(extension))
    {
        return Err(params_error(format!("{field} must end with .{extension}")));
    }
    Ok(path)
}

fn normalize_dir_path(field: &str, value: &str) -> Result<PathBuf, ErrorData> {
    let value = normalize_path_text(field, value)?;
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(params_error(format!("{field} must be an absolute path")));
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(params_error(format!("{field} must not contain '..'")));
    }
    Ok(path)
}

fn normalize_path_text(field: &str, value: &str) -> Result<String, ErrorData> {
    let value = value.trim();
    if value.is_empty() {
        return Err(params_error(format!("{field} must not be empty")));
    }
    if value.len() > MAX_PATH_BYTES {
        return Err(params_error(format!(
            "{field} must be <= {MAX_PATH_BYTES} bytes"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(params_error(format!(
            "{field} must not contain control characters"
        )));
    }
    Ok(value.to_owned())
}

fn normalize_query(value: &str) -> Result<String, ErrorData> {
    let value = value.trim();
    if value.is_empty() {
        return Err(params_error("query must not be empty"));
    }
    if value.len() > MAX_QUERY_BYTES {
        return Err(params_error(format!(
            "query must be <= {MAX_QUERY_BYTES} bytes"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(params_error("query must not contain control characters"));
    }
    Ok(value.to_owned())
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

fn validate_sha256(field: &str, value: &str) -> Result<String, ErrorData> {
    let value = normalize_required_text(field, value)?;
    if value.len() != 64 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(params_error(format!(
            "{field} must be a SHA-256 hex digest"
        )));
    }
    Ok(value.to_ascii_lowercase())
}

fn decode_json_row<T>(bytes: &[u8], row_key: &str) -> Result<T, ErrorData>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_slice::<T>(bytes).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_CORRUPTED,
            format!("decode ContextGraph bridge row {row_key}: {error}"),
        )
    })
}

fn compact_json(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "<json-encode-error>".to_owned())
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

fn len_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn len_to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn params_error(message: impl Into<String>) -> ErrorData {
    mcp_error(error_codes::TOOL_PARAMS_INVALID, message)
}

fn default_profile_id() -> String {
    EVERQUEST_PROFILE_ID.to_owned()
}

fn default_contextgraph_command() -> String {
    "context-graph-mcp".to_owned()
}

const fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

const fn default_importance() -> f64 {
    DEFAULT_IMPORTANCE
}

const fn default_top_k() -> u32 {
    DEFAULT_TOP_K
}

const fn default_require_provenance() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_raw_session_key_but_allows_session_hash_key() {
        let safe = json!({"session_id_sha256": "abc"});
        assert!(!contains_forbidden_raw_payload(&safe));
        let unsafe_payload = json!({"session_id": "abc"});
        assert!(contains_forbidden_raw_payload(&unsafe_payload));
    }

    #[test]
    fn extracts_episode_hash_citation_from_search_content() {
        let value = json!({
            "results": [{
                "fingerprintId": "fp-1",
                "content": "source_episode_id=traj.step1 source_export_sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            }]
        });
        let citations = extract_citations(&value);
        assert_eq!(citations.len(), 1);
        assert_eq!(citations[0].fingerprint_id, "fp-1");
        assert_eq!(citations[0].source_episode_id, "traj.step1");
    }

    #[test]
    fn rejects_invalid_episode_hash_citation_markers() {
        let value = json!({
            "results": [{
                "fingerprintId": "fp-1",
                "content": "source_episode_id=traj.step1 source_export_sha256=not-a-hash"
            }]
        });
        let citations = extract_citations(&value);
        assert!(citations.is_empty());
    }

    #[test]
    fn rejects_relative_contextgraph_storage_path() {
        let error = normalize_contextgraph_config(
            "context-graph-mcp",
            "relative\\db",
            None,
            true,
            DEFAULT_TIMEOUT_MS,
        )
        .unwrap_err();
        assert_eq!(
            error.data.as_ref().and_then(|data| data.get("code")),
            Some(&json!(error_codes::TOOL_PARAMS_INVALID))
        );
    }

    #[test]
    fn ingest_row_key_is_hash_scoped_for_duplicate_detection() {
        let key = ingest_row_key(
            EVERQUEST_PROFILE_ID,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "trajectory.transition",
        );
        assert!(key.contains("everquest/contextgraph_ingest/v1/everquest.live/"));
        assert!(key.ends_with("/trajectory.transition"));
    }
}
