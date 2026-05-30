use super::{
    ErrorData, Implementation, ServerCapabilities, ServerHandler, ServerInfo, SynapseService,
    mcp_error, tool_handler,
};

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SynapseService {
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::CallToolResult, ErrorData> {
        let tool_name = request.name.to_string();
        let context = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        match self.tool_router.call(context).await {
            Ok(result) => Ok(result),
            Err(error) if error.data.is_none() && error.message == "tool not found" => {
                Err(mcp_error(
                    synapse_core::error_codes::TOOL_NOT_FOUND,
                    format!("tool not found: {tool_name}"),
                ))
            }
            Err(error)
                if error.data.is_none() && error.code == rmcp::model::ErrorCode::INVALID_PARAMS =>
            {
                Err(mcp_error(
                    synapse_core::error_codes::TOOL_PARAMS_INVALID,
                    error.message.to_string(),
                ))
            }
            Err(error) => Err(error),
        }
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, ErrorData> {
        // Normalize schemas before they reach the client: schemars emits a bare
        // boolean `true` schema for `serde_json::Value` fields, which strict MCP
        // clients reject (failing the whole tools/list). See
        // `super::schema_sanitize`.
        let tools = super::schema_sanitize::sanitize_tools(self.tool_router.list_all());
        Ok(rmcp::model::ListToolsResult {
            tools,
            meta: None,
            next_cursor: None,
        })
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "synapse-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(self.instructions())
    }
}
