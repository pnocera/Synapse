use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use synapse_core::error_codes;
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;
use tempfile::TempDir;

#[tokio::test]
async fn act_scroll_schema_defaults_recording_and_edges_fsv() -> anyhow::Result<()> {
    let log_dir = TempDir::new()?;
    let mut client = StdioMcpClient::launch_and_init_with_env(
        Some(log_dir.path()),
        &[("SYNAPSE_MCP_RECORDING_BACKEND", "1")],
    )
    .await?;
    let resp = client.tools_list().await?;
    let tools = resp
        .get("tools")
        .and_then(Value::as_array)
        .context("tools array missing")?;
    assert_act_scroll_schema(tools)?;
    call_act_scroll_happy_and_edges(&mut client).await?;

    assert!(client.shutdown().await?.success());
    let logs = read_logs(log_dir.path())?;
    assert_recording_log_readbacks(&logs)?;
    Ok(())
}

fn assert_act_scroll_schema(tools: &[Value]) -> anyhow::Result<()> {
    let act_scroll = tools
        .iter()
        .find(|tool| tool.get("name") == Some(&Value::String("act_scroll".to_owned())))
        .context("act_scroll tool missing")?;
    let schema = &act_scroll["inputSchema"];
    println!(
        "source_of_truth=tools_list tool=act_scroll edge=schema before=tool_count:{}",
        tools.len()
    );
    println!(
        "source_of_truth=tools_list tool=act_scroll edge=defaults after=dy:{} dx:{} smooth:{} additionalProperties:{}",
        schema["properties"]["dy"]["default"],
        schema["properties"]["dx"]["default"],
        schema["properties"]["smooth"]["default"],
        schema["additionalProperties"]
    );
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["dy"]["default"], 0);
    assert_eq!(schema["properties"]["dx"]["default"], 0);
    assert_eq!(schema["properties"]["smooth"]["default"], false);
    assert_scroll_at_schema_is_closed(schema);

    let projection = json!({
        "name": act_scroll["name"],
        "description": act_scroll["description"],
        "inputSchema": act_scroll["inputSchema"],
        "outputSchemaRoot": schema_root(act_scroll.get("outputSchema")),
    });
    insta::assert_json_snapshot!("m2_act_scroll_tool", projection);
    Ok(())
}

fn assert_scroll_at_schema_is_closed(schema: &Value) {
    let schema_text = schema.to_string();
    assert!(schema_text.contains("\"ActScrollPoint\""));
    assert!(schema_text.contains("\"additionalProperties\":false"));
    assert!(schema_text.contains("\"x\""));
    assert!(schema_text.contains("\"y\""));
}

async fn call_act_scroll_happy_and_edges(client: &mut StdioMcpClient) -> anyhow::Result<()> {
    println!("source_of_truth=mcp_act_scroll edge=empty_noop before=params:{{}}");
    let empty = client.tools_call("act_scroll", json!({})).await?;
    let response: ActScrollWireResponse = structured(&empty)?;
    println!(
        "source_of_truth=mcp_act_scroll edge=empty_noop after=ok:{} dy:{} dx:{} smooth:{} scrolled:{} backend_used:{} elapsed_ms:{} expected_sequence:<empty>",
        response.ok,
        response.dy,
        response.dx,
        response.smooth,
        response.scrolled,
        response.backend_used,
        response.elapsed_ms
    );
    assert!(response.ok);
    assert_eq!(response.dy, 0);
    assert_eq!(response.dx, 0);
    assert!(!response.smooth);
    assert!(!response.scrolled);
    assert_eq!(response.backend_used, "none");

    println!("source_of_truth=mcp_act_scroll edge=wheel_xy before=dy:-3 dx:1 at:(5,6)");
    let wheel_xy = client
        .tools_call(
            "act_scroll",
            json!({"dy": -3, "dx": 1, "at": {"x": 5, "y": 6}}),
        )
        .await?;
    let response: ActScrollWireResponse = structured(&wheel_xy)?;
    println!(
        "source_of_truth=mcp_act_scroll edge=wheel_xy after=ok:{} dy:{} dx:{} smooth:{} scrolled:{} backend_used:{} elapsed_ms:{} expected_sequence:mouse_scroll:dy=-3:dx=1:at=screen(5,6)",
        response.ok,
        response.dy,
        response.dx,
        response.smooth,
        response.scrolled,
        response.backend_used,
        response.elapsed_ms
    );
    assert!(response.ok);
    assert_eq!(response.dy, -3);
    assert_eq!(response.dx, 1);
    assert!(!response.smooth);
    assert!(response.scrolled);
    assert_eq!(response.backend_used, "software");

    call_act_scroll_error_edges(client).await
}

async fn call_act_scroll_error_edges(client: &mut StdioMcpClient) -> anyhow::Result<()> {
    assert_error_code(
        client,
        "smooth_unsupported",
        "smooth:true dy:1",
        json!({"dy": 1, "smooth": true}),
        error_codes::ACTION_BACKEND_UNAVAILABLE,
    )
    .await?;
    assert_error_code(
        client,
        "extra_property",
        "junk:true",
        json!({"dy": 1, "junk": true}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await?;
    assert_error_code(
        client,
        "invalid_at_shape",
        "at:{x:1,y:2,junk:true}",
        json!({"dy": 1, "at": {"x": 1, "y": 2, "junk": true}}),
        error_codes::TOOL_PARAMS_INVALID,
    )
    .await
}

async fn assert_error_code(
    client: &mut StdioMcpClient,
    edge: &str,
    before: &str,
    args: Value,
    expected_code: &'static str,
) -> anyhow::Result<()> {
    println!("source_of_truth=mcp_act_scroll edge={edge} before={before}");
    let error = client.tools_call_error("act_scroll", args).await?;
    println!("source_of_truth=mcp_act_scroll edge={edge} after={error}");
    assert_eq!(error_code(&error), Some(expected_code));
    Ok(())
}

fn assert_recording_log_readbacks(logs: &str) -> anyhow::Result<()> {
    let readbacks = recording_readbacks(logs)?;
    assert_readback(&readbacks, "empty_noop", "", 0)?;
    assert_readback(
        &readbacks,
        "wheel_xy",
        "mouse_scroll:dy=-3:dx=1:at=screen(5,6)",
        1,
    )?;
    println!(
        "source_of_truth=recording_log tool=act_scroll edge=failed_edges after_readback_count={} expected_successful_readbacks=2",
        readbacks.len()
    );
    assert_eq!(readbacks.len(), 2);
    Ok(())
}

fn assert_readback(
    readbacks: &[RecordingReadback],
    edge: &str,
    expected_sequence: &str,
    expected_count: u64,
) -> anyhow::Result<()> {
    let readback = readbacks
        .iter()
        .find(|readback| {
            readback.event_sequence == expected_sequence
                && readback.new_event_count == expected_count
        })
        .with_context(|| {
            format!("{edge} act_scroll recording readback missing expected sequence")
        })?;
    println!(
        "source_of_truth=recording_log tool=act_scroll edge={edge} after_event_sequence={} new_event_count={}",
        readback.event_sequence, readback.new_event_count
    );
    Ok(())
}

#[derive(serde::Deserialize)]
struct ActScrollWireResponse {
    ok: bool,
    dy: i32,
    dx: i32,
    smooth: bool,
    scrolled: bool,
    backend_used: String,
    elapsed_ms: u32,
}

#[derive(Debug)]
struct RecordingReadback {
    event_sequence: String,
    new_event_count: u64,
}

fn structured<T: DeserializeOwned>(resp: &Value) -> anyhow::Result<T> {
    serde_json::from_value(resp["structuredContent"].clone()).context("decode structuredContent")
}

fn error_code(error: &Value) -> Option<&str> {
    error
        .get("data")
        .and_then(|data| data.get("code"))
        .and_then(Value::as_str)
}

fn schema_root(value: Option<&Value>) -> Value {
    let Some(value) = value else {
        return Value::Null;
    };
    json!({
        "title": value.get("title"),
        "type": value.get("type"),
        "required": value.get("required"),
        "additionalProperties": value.get("additionalProperties"),
    })
}

fn read_logs(path: &std::path::Path) -> anyhow::Result<String> {
    let mut logs = String::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        if entry.metadata()?.is_file() {
            logs.push_str(&std::fs::read_to_string(entry.path())?);
        }
    }
    Ok(logs)
}

fn recording_readbacks(logs: &str) -> anyhow::Result<Vec<RecordingReadback>> {
    let mut readbacks = Vec::new();
    for line in logs.lines().filter(|line| !line.trim().is_empty()) {
        let value: Value = serde_json::from_str(line)?;
        let fields = &value["fields"];
        if fields.get("code").and_then(Value::as_str) != Some("M2_ACT_SCROLL_RECORDING_READBACK") {
            continue;
        }
        let event_sequence = fields
            .get("event_sequence")
            .and_then(Value::as_str)
            .context("recording readback missing event_sequence")?
            .to_owned();
        let new_event_count = fields
            .get("new_event_count")
            .and_then(Value::as_u64)
            .context("recording readback missing new_event_count")?;
        readbacks.push(RecordingReadback {
            event_sequence,
            new_event_count,
        });
    }
    Ok(readbacks)
}
