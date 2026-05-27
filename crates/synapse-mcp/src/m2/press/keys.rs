use std::collections::HashSet;

use rmcp::ErrorData;
use synapse_action::ActionError;
use synapse_core::{Key, KeyCode, error_codes};

use super::action_error_to_mcp;
use crate::m1::mcp_error;

const MODIFIER_ORDER: [&str; 4] = ["ctrl", "shift", "alt", "super"];

pub(in crate::m2::press) fn normalized_keys(raw_keys: &[String]) -> Result<Vec<Key>, ErrorData> {
    if raw_keys.is_empty() {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "act_press keys must contain at least one key",
        ));
    }

    let mut seen = HashSet::new();
    let mut names = Vec::with_capacity(raw_keys.len());
    for raw_key in raw_keys {
        let name = canonical_key_name(raw_key)?;
        if !seen.insert(name.clone()) {
            return Err(mcp_error(
                error_codes::TOOL_PARAMS_INVALID,
                format!("act_press duplicate key '{name}'"),
            ));
        }
        names.push(name);
    }

    let mut ordered = Vec::with_capacity(names.len());
    for modifier in MODIFIER_ORDER {
        if names.iter().any(|name| name == modifier) {
            ordered.push(key(modifier));
        }
    }
    for name in names
        .iter()
        .filter(|name| !MODIFIER_ORDER.contains(&name.as_str()))
    {
        ordered.push(key(name));
    }
    Ok(ordered)
}

fn canonical_key_name(raw_key: &str) -> Result<String, ErrorData> {
    let lowered = raw_key.trim().to_ascii_lowercase();
    let key = match lowered.as_str() {
        "" => {
            return Err(mcp_error(
                error_codes::TOOL_PARAMS_INVALID,
                "act_press key names must be non-empty",
            ));
        }
        "control" => "ctrl",
        "escape" => "esc",
        "return" => "enter",
        "backtick" | "grave" | "graveaccent" | "keyboardgraveaccent" => "`",
        "arrowup" => "up",
        "arrowdown" => "down",
        "arrowleft" => "left",
        "arrowright" => "right",
        "win" | "windows" | "meta" => "super",
        "pgup" => "pageup",
        "pgdn" => "pagedown",
        other => other,
    };

    if is_allowed_key_name(key) {
        Ok(key.to_owned())
    } else {
        Err(action_error_to_mcp(&ActionError::UnsupportedKey {
            detail: format!("act_press unsupported key '{raw_key}'"),
        }))
    }
}

fn is_allowed_key_name(key: &str) -> bool {
    if key.len() == 1 && key.as_bytes()[0].is_ascii_alphanumeric() {
        return true;
    }
    if let Some(number) = key
        .strip_prefix('f')
        .and_then(|suffix| suffix.parse::<u8>().ok())
    {
        return (1..=24).contains(&number);
    }
    matches!(
        key,
        "`" | "alt"
            | "backspace"
            | "ctrl"
            | "delete"
            | "down"
            | "end"
            | "enter"
            | "esc"
            | "home"
            | "insert"
            | "left"
            | "pagedown"
            | "pageup"
            | "right"
            | "shift"
            | "space"
            | "super"
            | "tab"
            | "up"
    )
}

pub(in crate::m2::press) fn key(value: &str) -> Key {
    Key {
        code: KeyCode::Named {
            value: value.to_owned(),
        },
        use_scancode: false,
    }
}
