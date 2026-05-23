use anyhow::Context;
use serde_json::{Value, json};
use synapse_test_utils::stdio_mcp_client::StdioMcpClient;

const EXPECTED_M2_TOOL_NAMES: &[&str] = &[
    "act_aim",
    "act_click",
    "act_clipboard",
    "act_drag",
    "act_pad",
    "act_press",
    "act_scroll",
    "act_type",
    "find",
    "health",
    "observe",
    "read_text",
    "release_all",
    "set_capture_target",
    "set_perception_mode",
];

const M2_ACTION_TOOL_NAMES: &[&str] = &[
    "act_aim",
    "act_click",
    "act_clipboard",
    "act_drag",
    "act_pad",
    "act_press",
    "act_scroll",
    "act_type",
    "release_all",
];

#[tokio::test]
async fn m2_tools_list_contains_exact_sorted_surface_fsv() -> anyhow::Result<()> {
    let mut client = StdioMcpClient::launch_and_init().await?;
    let resp = client.tools_list().await?;
    let tools = resp
        .get("tools")
        .and_then(Value::as_array)
        .context("tools array missing")?;

    let mut names = tools
        .iter()
        .map(|tool| {
            tool.get("name")
                .and_then(Value::as_str)
                .context("tool name missing")
                .map(str::to_owned)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    names.sort();
    println!("source_of_truth=tools_list edge=m2 final_names={names:?}");
    assert_eq!(names, EXPECTED_M2_TOOL_NAMES);

    let m2_action_tools = tools
        .iter()
        .filter(|tool| {
            tool.get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| M2_ACTION_TOOL_NAMES.contains(&name))
        })
        .collect::<Vec<_>>();
    assert_eq!(m2_action_tools.len(), M2_ACTION_TOOL_NAMES.len());
    for tool in &m2_action_tools {
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("<missing>");
        assert_closed_schema(&tool["inputSchema"], &format!("{name}.inputSchema"));
        if let Some(output) = tool.get("outputSchema") {
            assert_closed_schema(output, &format!("{name}.outputSchema"));
        }
    }
    println!(
        "source_of_truth=schema_closed edge=m2 after=checked_tools:{}",
        m2_action_tools.len()
    );

    let default_rows = default_rows();
    assert_eq!(default_rows.len(), 23);
    for (tool_name, field, expected) in &default_rows {
        let actual = schema_default(tools, tool_name, field)?;
        println!(
            "source_of_truth=schema_default tool={tool_name} field={field} before={} after={}",
            printable_value(expected),
            printable_value(actual)
        );
        assert_eq!(
            actual, expected,
            "{tool_name}.{field} schema default must match M2 default table"
        );
    }
    println!(
        "source_of_truth=schema_default edge=m2 after=checked_defaults:{}",
        default_rows.len()
    );

    let mut projection = tools
        .iter()
        .map(|tool| {
            let name = tool
                .get("name")
                .and_then(Value::as_str)
                .context("tool name missing")?;
            Ok((
                name.to_owned(),
                json!({
                    "name": tool["name"],
                    "description": tool["description"],
                    "inputSchema": tool["inputSchema"],
                    "outputSchema": tool.get("outputSchema").unwrap_or(&Value::Null),
                }),
            ))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    projection.sort_by(|left, right| left.0.cmp(&right.0));
    let schemas = projection
        .into_iter()
        .map(|(_name, schema)| schema)
        .collect::<Vec<_>>();
    insta::assert_json_snapshot!("m2_tools_list", schemas);

    assert!(client.shutdown().await?.success());
    Ok(())
}

fn default_rows() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        ("act_click", "curve", json!("natural")),
        ("act_click", "duration_ms", json!(50)),
        ("act_click", "button", json!("left")),
        ("act_click", "clicks", json!(1)),
        ("act_click", "use_invoke_pattern", json!(true)),
        ("act_click", "backend", json!("auto")),
        ("act_type", "dynamics", json!("natural")),
        ("act_type", "backend", json!("auto")),
        ("act_type", "press_enter_after", json!(false)),
        ("act_type", "use_scancodes", json!(false)),
        ("act_press", "hold_ms", json!(33)),
        ("act_press", "backend", json!("auto")),
        ("act_aim", "style", json!("snap")),
        ("act_aim", "deadline_ms", json!(80)),
        ("act_drag", "curve", json!("natural")),
        ("act_drag", "duration_ms", json!(200)),
        ("act_drag", "button", json!("left")),
        ("act_scroll", "dy", json!(0)),
        ("act_scroll", "dx", json!(0)),
        ("act_scroll", "smooth", json!(false)),
        ("act_pad", "pad_id", json!(0)),
        ("act_pad", "backend", json!("vigem")),
        ("act_clipboard", "format", json!("unicode")),
    ]
}

fn schema_default<'a>(
    tools: &'a [Value],
    tool_name: &str,
    field: &str,
) -> anyhow::Result<&'a Value> {
    tool_by_name(tools, tool_name)?
        .get("inputSchema")
        .and_then(|schema| schema.get("properties"))
        .and_then(|properties| properties.get(field))
        .and_then(|property| property.get("default"))
        .with_context(|| format!("{tool_name}.{field} default missing from tools/list schema"))
}

fn tool_by_name<'a>(tools: &'a [Value], tool_name: &str) -> anyhow::Result<&'a Value> {
    tools
        .iter()
        .find(|tool| tool.get("name").and_then(Value::as_str) == Some(tool_name))
        .with_context(|| format!("{tool_name} missing from tools/list"))
}

fn printable_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}

fn assert_closed_schema(value: &Value, path: &str) {
    match value {
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("object") {
                assert_eq!(
                    object.get("additionalProperties"),
                    Some(&Value::Bool(false)),
                    "object schema at {path} must set additionalProperties:false"
                );
            }
            for (key, child) in object {
                assert_closed_schema(child, &format!("{path}.{key}"));
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                assert_closed_schema(child, &format!("{path}[{index}]"));
            }
        }
        _ => {}
    }
}
