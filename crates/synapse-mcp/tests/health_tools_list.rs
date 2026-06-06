use anyhow::Context;
use serde_json::Value;
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;

#[tokio::test]
async fn health_and_value_tools_appear_in_tools_list_with_schema() -> anyhow::Result<()> {
    let mut client = StdioMcpClient::launch_and_init().await?;
    let resp = client.tools_list().await?;
    let tools = resp
        .get("tools")
        .and_then(Value::as_array)
        .context("tools array missing")?;
    let health_tool = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&Value::String("health".to_owned())))
        .context("health tool missing")?;

    assert_eq!(health_tool["description"], "Return server health");
    assert_eq!(health_tool["inputSchema"]["type"], "object");
    let set_value_tool = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&Value::String("act_set_value".to_owned())))
        .context("act_set_value tool missing")?;
    assert_eq!(set_value_tool["inputSchema"]["type"], "object");
    assert_eq!(
        set_value_tool["inputSchema"]["additionalProperties"],
        Value::Bool(false)
    );
    assert!(
        set_value_tool["inputSchema"]["properties"]["element_id"]
            .get("$ref")
            .and_then(Value::as_str)
            .is_some_and(|reference| reference.contains("ElementId"))
    );
    assert_eq!(
        set_value_tool["inputSchema"]["properties"]["text"]["type"],
        "string"
    );

    let click_tool = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&Value::String("act_click".to_owned())))
        .context("act_click tool missing")?;
    assert_eq!(
        click_tool["inputSchema"]["properties"]["coordinate_fallback_on_unsupported"]["default"],
        Value::Bool(true)
    );
    assert!(
        click_tool["description"]
            .as_str()
            .is_some_and(|description| description.contains("coordinate_fallback_on_unsupported"))
    );
    assert!(
        client
            .raw_received()
            .iter()
            .any(|line| line.contains("\"tools\"") && line.contains("\"act_set_value\""))
    );
    let status = client.shutdown().await?;
    assert!(status.success());
    Ok(())
}
