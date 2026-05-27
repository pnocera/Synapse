use super::{
    ErrorData, FindParams, FindResponse, Health, Json, ObserveParams, Parameters, ReadTextParams,
    SetCaptureTargetParams, SetCaptureTargetResponse, SetPerceptionModeParams,
    SetPerceptionModeResponse, SynapseService, assemble_observation, empty_input_schema,
    find_in_state, read_text_in_state, set_capture_target_in_state, set_perception_mode_in_state,
    tool, tool_router,
};

#[tool_router(router = m1_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(description = "Return server health", input_schema = empty_input_schema())]
    pub async fn health(&self) -> Json<Health> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "health",
            "tool.invocation kind=health"
        );
        Json(self.health_payload())
    }

    #[tool(description = "Returns structured state of the focused window and surrounding context")]
    pub async fn observe(
        &self,
        params: Parameters<ObserveParams>,
    ) -> Result<Json<synapse_core::Observation>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "observe",
            "tool.invocation kind=observe"
        );
        let state = self.m1_state()?;
        let mut observation = assemble_observation(&state, &params.0)?;
        drop(state);

        self.resolve_observation_profile(&mut observation);

        let mut state = self.m1_state()?;
        state.last_observed_foreground = Some(observation.foreground.clone());
        drop(state);
        Ok(Json(observation))
    }

    #[tool(description = "Search visible accessibility nodes and detected entities")]
    pub async fn find(
        &self,
        params: Parameters<FindParams>,
    ) -> Result<Json<FindResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "find",
            "tool.invocation kind=find"
        );
        let state = self.m1_state()?;
        find_in_state(&state, &params.0).map(Json)
    }

    #[tool(description = "OCR text from a screen region or visible element")]
    pub async fn read_text(
        &self,
        params: Parameters<ReadTextParams>,
    ) -> Result<Json<synapse_core::OcrResult>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "read_text",
            "tool.invocation kind=read_text"
        );
        let state = self.m1_state()?;
        read_text_in_state(&state, params.0).map(Json)
    }

    #[tool(description = "Set the active capture target")]
    pub async fn set_capture_target(
        &self,
        params: Parameters<SetCaptureTargetParams>,
    ) -> Result<Json<SetCaptureTargetResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "set_capture_target",
            "tool.invocation kind=set_capture_target"
        );
        let mut state = self.m1_state()?;
        set_capture_target_in_state(&mut state, params.0).map(Json)
    }

    #[tool(description = "Set the active perception mode")]
    pub async fn set_perception_mode(
        &self,
        params: Parameters<SetPerceptionModeParams>,
    ) -> Result<Json<SetPerceptionModeResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "set_perception_mode",
            "tool.invocation kind=set_perception_mode"
        );
        let mut state = self.m1_state()?;
        set_perception_mode_in_state(&mut state, &params.0).map(Json)
    }
}

impl SynapseService {
    fn resolve_observation_profile(&self, observation: &mut synapse_core::Observation) {
        let foreground = synapse_profiles::ForegroundWindow {
            exe: non_empty(&observation.foreground.process_name),
            title: non_empty(&observation.foreground.window_title),
            steam_appid: observation.foreground.steam_appid,
            window_class: None,
        };

        let Ok(runtime) = self.profile_runtime() else {
            tracing::warn!(
                code = "PROFILE_FOREGROUND_RESOLUTION_SKIPPED",
                "profile runtime unavailable while resolving observed foreground"
            );
            return;
        };

        match runtime.resolve_foreground(&foreground) {
            Ok(Some(resolution)) => {
                tracing::info!(
                    code = "PROFILE_FOREGROUND_MATCHED",
                    profile_id = %resolution.profile_id,
                    rank = resolution.rank_name,
                    "observed foreground matched profile"
                );
                observation.foreground.profile_id = Some(resolution.profile_id);
            }
            Ok(None) => {
                tracing::debug!(
                    code = "PROFILE_FOREGROUND_UNMATCHED",
                    "observed foreground did not match a loaded profile"
                );
            }
            Err(error) => {
                tracing::warn!(
                    code = "PROFILE_FOREGROUND_RESOLUTION_FAILED",
                    error = %error,
                    "profile resolver failed for observed foreground"
                );
            }
        }
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}
