use std::collections::BTreeMap;

use chrono::Utc;
use rmcp::ErrorData;
use synapse_core::{
    AccessibleNode, AudioContext, DetectedEntity, FocusedElement, ForegroundContext, HudReadings,
    PerceptionMode, Rect, SensorStatus, UiaPattern, element_id, entity_id, error_codes,
};
use synapse_perception::ObservationInput;

use crate::m1::mcp_error;

pub fn synthetic_notepad_input() -> ObservationInput {
    let at = Utc::now();
    let focused_id = element_id(0x1234, "0000002a00000001");
    let elements = vec![
        node(0, 0, "Notepad", "Window", false),
        node(1, 1, "Document", "Edit", true),
        node(2, 1, "File", "MenuItem", false),
        node(3, 1, "Edit", "MenuItem", false),
        node(4, 1, "View", "MenuItem", false),
        node(5, 1, "Status", "Text", false),
    ];
    let mut latency = BTreeMap::new();
    latency.insert("a11y".to_owned(), 1.25);
    latency.insert("capture".to_owned(), 0.50);
    ObservationInput {
        foreground: ForegroundContext {
            hwnd: 0x1234,
            pid: 44,
            process_name: "notepad.exe".to_owned(),
            process_path: "C:\\Windows\\System32\\notepad.exe".to_owned(),
            window_title: "manual-fsv.txt - Notepad".to_owned(),
            window_bounds: Rect {
                x: 10,
                y: 20,
                w: 800,
                h: 600,
            },
            monitor_index: 0,
            dpi_scale: 1.0,
            profile_id: None,
            steam_appid: None,
            is_fullscreen: false,
            is_dwm_composed: true,
        },
        focused: Some(FocusedElement {
            element_id: focused_id,
            name: "Document".to_owned(),
            role: "Edit".to_owned(),
            automation_id: Some("15".to_owned()),
            bbox: Rect {
                x: 12,
                y: 80,
                w: 760,
                h: 480,
            },
            enabled: true,
            patterns: vec![UiaPattern::Text, UiaPattern::Value],
            value: Some("Synthetic Synapse text".to_owned()),
            selected_text: None,
        }),
        elements,
        entities: vec![DetectedEntity {
            entity_id: entity_id(9),
            track_id: 9,
            class_label: "cursor".to_owned(),
            bbox: Rect {
                x: 40,
                y: 90,
                w: 8,
                h: 20,
            },
            confidence: 0.80,
            first_seen_at: at,
            last_seen_at: at,
            velocity_px_per_s: None,
        }],
        hud: HudReadings::default(),
        audio: AudioContext::default(),
        recent_events: Vec::new(),
        clipboard_summary: None,
        fs_recent: Vec::new(),
        sensor_latency_ms: latency,
        a11y_status: SensorStatus::Healthy,
        capture_status: SensorStatus::Healthy,
        detection_status: SensorStatus::Disabled,
        audio_status: SensorStatus::Disabled,
        mode_override: None,
    }
}

fn node(sequence: u32, depth: u32, name: &str, role: &str, focused: bool) -> AccessibleNode {
    let depth_i32 = i32::try_from(depth).unwrap_or(0);
    let sequence_i32 = i32::try_from(sequence).unwrap_or(0);
    AccessibleNode {
        element_id: element_id(0x1234, &format!("0000002a{sequence:08x}")),
        parent: (depth > 0).then(|| element_id(0x1234, "0000002a00000000")),
        name: name.to_owned(),
        role: role.to_owned(),
        automation_id: None,
        bbox: Rect {
            x: 10 + depth_i32,
            y: 20 + sequence_i32.saturating_mul(10),
            w: 100,
            h: 30,
        },
        enabled: true,
        focused,
        patterns: Vec::new(),
        children_count: 0,
        depth,
    }
}

#[cfg(not(windows))]
pub fn platform_input(_depth: u32, _mode: PerceptionMode) -> Result<ObservationInput, ErrorData> {
    Err(mcp_error(
        error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE,
        "UIA foreground window lookup requires Windows",
    ))
}

#[cfg(windows)]
pub fn platform_input(depth: u32, mode: PerceptionMode) -> Result<ObservationInput, ErrorData> {
    let root = synapse_a11y::focused_window().map_err(|err| a11y_error(&err))?;
    let tree = synapse_a11y::snapshot(&root, depth).map_err(|err| a11y_error(&err))?;
    let hwnd = tree
        .root
        .parts()
        .map_err(|err| mcp_error(error_codes::OBSERVE_INTERNAL, err.to_string()))?
        .hwnd;
    let foreground = windows_foreground_context(hwnd)?;
    let focused = tree
        .nodes
        .iter()
        .find(|node| node.focused)
        .or_else(|| tree.nodes.first())
        .map(focused_from_node);
    let mut input = ObservationInput::new(foreground);
    input.focused = focused;
    input.elements = tree.nodes;
    input.a11y_status = SensorStatus::Healthy;
    input.capture_status = SensorStatus::Unavailable;
    if mode != PerceptionMode::Auto {
        input.mode_override = Some(mode);
    }
    Ok(input)
}

#[cfg(windows)]
fn a11y_error(err: &synapse_a11y::A11yError) -> ErrorData {
    match err {
        synapse_a11y::A11yError::NoForeground { .. }
        | synapse_a11y::A11yError::NotAvailable { .. } => mcp_error(
            error_codes::OBSERVE_NO_PERCEPTION_AVAILABLE,
            err.to_string(),
        ),
        _ => mcp_error(error_codes::OBSERVE_INTERNAL, err.to_string()),
    }
}

#[cfg(windows)]
fn focused_from_node(node: &AccessibleNode) -> FocusedElement {
    FocusedElement {
        element_id: node.element_id.clone(),
        name: node.name.clone(),
        role: node.role.clone(),
        automation_id: node.automation_id.clone(),
        bbox: node.bbox,
        enabled: node.enabled,
        patterns: node.patterns.clone(),
        value: None,
        selected_text: None,
    }
}

#[cfg(windows)]
fn windows_foreground_context(hwnd: i64) -> Result<ForegroundContext, ErrorData> {
    synapse_a11y::foreground_context(hwnd).map_err(|err| a11y_error(&err))
}
