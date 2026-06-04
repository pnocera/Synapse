use std::sync::Arc;

use serde_json::json;
use synapse_action::{ActionBackend, ActionEmitter, RecordingBackend};
use tokio_util::sync::CancellationToken;

use super::{
    act_click_with_handle,
    schema::{
        ActClickParams, ActClickPointTarget, ActClickTarget, ClickVelocityProfile,
        default_click_backend, default_click_button, default_click_count,
        default_click_duration_ms, default_click_velocity_profile, default_use_invoke_pattern,
    },
};

#[tokio::test]
async fn coordinate_click_leaves_actor_held_state_empty() {
    let cancel = CancellationToken::new();
    let backend: Arc<dyn ActionBackend> = Arc::new(RecordingBackend::new());
    let (handle, snapshot_handle, join) =
        ActionEmitter::spawn_with_backend(cancel.clone(), backend);
    let before = match snapshot_handle.snapshot().await {
        Ok(snapshot) => snapshot,
        Err(err) => panic!("before snapshot failed: {err}"),
    };
    println!(
        "readback=act_click_actor edge=coordinate before=held_buttons:{:?} held_keys:{:?}",
        before.held_buttons, before.held_keys
    );
    let response = match act_click_with_handle(
        handle,
        None,
        ActClickParams {
            target: ActClickTarget::Point(ActClickPointTarget { x: 12, y: 34 }),
            button: default_click_button(),
            clicks: default_click_count(),
            modifiers: Vec::new(),
            velocity_profile: default_click_velocity_profile(),
            duration_ms: default_click_duration_ms(),
            hold_ms: super::schema::default_click_hold_ms(),
            backend: default_click_backend(),
            use_invoke_pattern: default_use_invoke_pattern(),
            deprecated_curve_alias_used: false,
        },
    )
    .await
    {
        Ok(response) => response,
        Err(err) => panic!("act_click failed: {err}"),
    };
    let after = match snapshot_handle.snapshot().await {
        Ok(snapshot) => snapshot,
        Err(err) => panic!("after snapshot failed: {err}"),
    };
    println!(
        "readback=act_click_actor edge=coordinate after=ok:{} backend_used:{} held_buttons:{:?} held_keys:{:?}",
        response.ok, response.backend_used, after.held_buttons, after.held_keys
    );
    assert!(response.ok);
    assert!(!response.used_invoke_pattern);
    assert_eq!(response.backend_used, "software");
    assert_eq!(response.press_hold_ms, 120);
    assert!(after.held_buttons.is_empty());
    assert!(after.held_keys.is_empty());
    cancel.cancel();
    let _final_snapshot = match join.await {
        Ok(snapshot) => snapshot,
        Err(err) => panic!("join failed: {err}"),
    };
}

#[test]
fn click_velocity_profile_accepts_hidden_legacy_curve_alias() {
    let new_name: ActClickParams = serde_json::from_value(json!({
        "target": {"x": 10, "y": 20},
        "velocity_profile": "linear"
    }))
    .expect("velocity_profile should parse");
    assert_eq!(new_name.velocity_profile, ClickVelocityProfile::Linear);
    assert!(!new_name.deprecated_curve_alias_used);

    let old_alias: ActClickParams = serde_json::from_value(json!({
        "target": {"x": 10, "y": 20},
        "curve": "ease_in_out"
    }))
    .expect("legacy curve alias should parse");
    assert_eq!(old_alias.velocity_profile, ClickVelocityProfile::EaseInOut);
    assert!(old_alias.deprecated_curve_alias_used);

    let conflict = serde_json::from_value::<ActClickParams>(json!({
        "target": {"x": 10, "y": 20},
        "velocity_profile": "linear",
        "curve": "natural"
    }))
    .expect_err("velocity_profile and curve together must fail closed");
    assert!(
        conflict
            .to_string()
            .contains("velocity_profile or deprecated curve"),
        "{conflict}"
    );
}
