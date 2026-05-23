use synapse_core::{AccessibleNode, DetectedEntity};

use crate::m1::{FindParams, FindResult, FindResultKind};

pub fn element_match(node: &AccessibleNode, params: &FindParams) -> Option<FindResult> {
    if params.in_window.is_some() && params.in_window.as_ref() != node.parent.as_ref() {
        return None;
    }
    if let Some(role) = &params.role
        && !node.role.eq_ignore_ascii_case(role)
    {
        return None;
    }
    if let Some(name_substring) = &params.name_substring
        && !contains_ascii_case(&node.name, name_substring)
    {
        return None;
    }
    if let Some(automation_id) = &params.automation_id
        && node.automation_id.as_deref() != Some(automation_id.as_str())
    {
        return None;
    }
    let mut score = 0.25;
    if let Some(query) = &params.query {
        if contains_ascii_case(&node.name, query)
            || contains_ascii_case(&node.role, query)
            || node
                .automation_id
                .as_deref()
                .is_some_and(|value| contains_ascii_case(value, query))
        {
            score += 0.65;
        } else if params.role.is_none()
            && params.name_substring.is_none()
            && params.automation_id.is_none()
        {
            return None;
        }
    }
    if node.focused {
        score += 0.1;
    }
    Some(FindResult {
        kind: FindResultKind::Element,
        element_id: Some(node.element_id.clone()),
        entity_id: None,
        name: Some(node.name.clone()),
        role: Some(node.role.clone()),
        automation_id: node.automation_id.clone(),
        class_label: None,
        bbox: node.bbox,
        score,
    })
}

pub fn entity_match(entity: &DetectedEntity, params: &FindParams) -> Option<FindResult> {
    let query = params.query.as_ref()?;
    contains_ascii_case(&entity.class_label, query).then_some(FindResult {
        kind: FindResultKind::Entity,
        element_id: None,
        entity_id: Some(entity.entity_id.clone()),
        name: None,
        role: None,
        automation_id: None,
        class_label: Some(entity.class_label.clone()),
        bbox: entity.bbox,
        score: entity.confidence,
    })
}

fn contains_ascii_case(value: &str, needle: &str) -> bool {
    value
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}
