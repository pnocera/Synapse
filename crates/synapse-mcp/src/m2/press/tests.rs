use std::{sync::Arc, time::Duration};

use synapse_action::{ActionEmitter, RecordedInput, RecordingBackend};
use synapse_core::{Action, Backend};
use tokio_util::sync::CancellationToken;

use super::{
    act_press_with_handle,
    keys::{key, normalized_keys},
    live::execute_live_press_sequence,
    record::event_sequence,
    schema::{ActPressParams, PressBackend, default_hold_ms, default_press_backend},
};

#[tokio::test]
async fn recording_backend_readback_orders_chord_and_default_hold() {
    let (handle, _snapshot_handle, _emitter) = ActionEmitter::channel();
    let recording = Arc::new(RecordingBackend::new());
    let params = ActPressParams {
        keys: vec!["shift".to_owned(), "ctrl".to_owned(), "s".to_owned()],
        hold_ms: default_hold_ms(),
        backend: default_press_backend(),
    };
    let before = recording.events();
    println!("readback=act_press_recording edge=ordered_chord before={before:?}");

    let response = act_press_with_handle(handle, Some(Arc::clone(&recording)), None, params)
        .await
        .unwrap_or_else(|error| panic!("act_press recording should succeed: {error}"));
    let after = recording.events();
    let sequence = event_sequence(&after);
    println!(
        "readback=act_press_recording edge=ordered_chord after={after:?} sequence={sequence} keys_pressed={}",
        response.keys_pressed
    );

    assert!(response.ok);
    assert_eq!(response.keys_pressed, 3);
    assert_eq!(
        sequence,
        "down:ctrl>down:shift>down:s>delay:33>up:s>up:shift>up:ctrl"
    );
}

#[tokio::test]
async fn live_press_sequence_leaves_actor_available_for_release_all_mid_hold() {
    let cancel = CancellationToken::new();
    let recording = Arc::new(RecordingBackend::new());
    let (handle, snapshot_handle, join) =
        ActionEmitter::spawn_with_backend(cancel.clone(), recording.clone());
    let keys = vec![key("a")];
    let started_events = recording.events();
    println!(
        "readback=act_press_live_sequence edge=mid_hold_release before_events={started_events:?}"
    );

    let press = tokio::spawn(execute_live_press_sequence(
        handle.clone(),
        keys,
        50,
        Backend::Software,
        None,
    ));
    let before_release = wait_for_held_key(&snapshot_handle, "a").await;
    println!(
        "readback=act_press_live_sequence edge=mid_hold_release before_release={before_release:?}"
    );

    handle
        .execute(Action::ReleaseAll)
        .await
        .unwrap_or_else(|error| panic!("release_all should execute during hold: {error}"));
    let after_release = snapshot_handle
        .snapshot()
        .await
        .unwrap_or_else(|error| panic!("snapshot after release_all should succeed: {error}"));
    println!(
        "readback=act_press_live_sequence edge=mid_hold_release after_release={after_release:?}"
    );
    assert!(after_release.held_keys.is_empty());

    press
        .await
        .unwrap_or_else(|error| panic!("press task should join: {error}"))
        .unwrap_or_else(|error| panic!("press task should tolerate prior release_all: {error}"));
    let final_events = recording.events();
    println!(
        "readback=act_press_live_sequence edge=mid_hold_release after_events={final_events:?}"
    );
    assert!(
        final_events
            .iter()
            .any(|event| matches!(event, RecordedInput::ReleaseAll { .. }))
    );

    cancel.cancel();
    let final_snapshot = join
        .await
        .unwrap_or_else(|error| panic!("emitter should join: {error}"));
    assert!(final_snapshot.held_keys.is_empty());
}

#[test]
fn defaults_are_issue_required_values() {
    assert_eq!(default_hold_ms(), 33);
    assert_eq!(default_press_backend(), PressBackend::Auto);
}

#[test]
fn normalized_keys_are_modifier_ordered() {
    let before = vec!["super".to_owned(), "s".to_owned(), "ctrl".to_owned()];
    println!("readback=act_press_keys edge=modifier_order before={before:?}");
    let after =
        normalized_keys(&before).unwrap_or_else(|error| panic!("keys should normalize: {error}"));
    let labels = after
        .iter()
        .map(|key| match &key.code {
            synapse_core::KeyCode::Named { value } => value.as_str(),
            _ => "",
        })
        .collect::<Vec<_>>();
    println!("readback=act_press_keys edge=modifier_order after={labels:?}");
    assert_eq!(labels, ["ctrl", "super", "s"]);
}

#[test]
fn normalized_keys_accept_vs_code_terminal_backtick_shortcut() {
    let before = vec!["ctrl".to_owned(), "`".to_owned()];
    println!("readback=act_press_keys edge=backtick_shortcut before={before:?}");
    let after = normalized_keys(&before)
        .unwrap_or_else(|error| panic!("backtick should normalize: {error}"));
    let labels = after
        .iter()
        .map(|key| match &key.code {
            synapse_core::KeyCode::Named { value } => value.as_str(),
            _ => "",
        })
        .collect::<Vec<_>>();
    println!("readback=act_press_keys edge=backtick_shortcut after={labels:?}");
    assert_eq!(labels, ["ctrl", "`"]);
}

#[test]
fn event_sequence_reads_recording_events() {
    let before = vec![
        RecordedInput::KeyDown { key: key("ctrl") },
        RecordedInput::DelayMs { ms: 33 },
        RecordedInput::KeyUp { key: key("ctrl") },
    ];
    let after = event_sequence(&before);
    println!("readback=act_press_recording edge=event_sequence before={before:?} after={after}");
    assert_eq!(after, "down:ctrl>delay:33>up:ctrl");
}

async fn wait_for_held_key(
    snapshot_handle: &synapse_action::ActionEmitterSnapshotHandle,
    key_name: &str,
) -> synapse_action::ActionStateSnapshot {
    for _ in 0..50 {
        let snapshot = snapshot_handle
            .snapshot()
            .await
            .unwrap_or_else(|error| panic!("snapshot should succeed: {error}"));
        if snapshot.held_keys.iter().any(|key| match &key.code {
            synapse_core::KeyCode::Named { value } => value == key_name,
            _ => false,
        }) {
            return snapshot;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("timed out waiting for held key {key_name}");
}
