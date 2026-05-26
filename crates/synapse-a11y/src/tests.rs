use std::time::Duration;

use proptest::prelude::*;
use synapse_core::{ElementId, element_id, error_codes};
use tokio::net::TcpListener;

use crate::*;

fn event(
    seq: u64,
    at_ms: u64,
    element: Option<ElementId>,
    kind: AccessibleEventKind,
) -> AccessibleEvent {
    AccessibleEvent {
        seq,
        at_ms,
        window_id: 0x1234,
        element_id: element,
        kind,
        name: None,
        value: None,
    }
}

#[test]
fn coalesce_empty_input_prints_before_after_state() {
    let before = Vec::<AccessibleEvent>::new();
    println!("readback=coalesced_events edge=empty before={before:?}");
    let after = coalesce_events(before, Duration::from_millis(50));
    println!("readback=coalesced_events edge=empty after={after:?}");
    assert!(after.is_empty());
}

#[test]
fn coalesce_same_key_within_window_keeps_latest_state() -> Result<(), Box<dyn std::error::Error>> {
    let id = ElementId::parse("0x1234:00000001")?;
    let mut first = event(1, 0, Some(id.clone()), AccessibleEventKind::NameChanged);
    first.name = Some("old".to_owned());
    let mut second = event(2, 49, Some(id), AccessibleEventKind::NameChanged);
    second.name = Some("new".to_owned());
    let before = vec![first, second];
    println!("readback=coalesced_events edge=within_50ms before={before:?}");
    let after = coalesce_events(before, Duration::from_millis(50));
    println!("readback=coalesced_events edge=within_50ms after={after:?}");
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].name.as_deref(), Some("new"));
    assert_eq!(after[0].seq, 2);
    Ok(())
}

#[test]
fn coalesce_exact_boundary_does_not_merge() -> Result<(), Box<dyn std::error::Error>> {
    let id = ElementId::parse("0x1234:00000002")?;
    let before = vec![
        event(1, 0, Some(id.clone()), AccessibleEventKind::FocusChanged),
        event(2, 50, Some(id), AccessibleEventKind::FocusChanged),
    ];
    println!("readback=coalesced_events edge=exact_50ms before={before:?}");
    let after = coalesce_events(before, Duration::from_millis(50));
    println!("readback=coalesced_events edge=exact_50ms after={after:?}");
    assert_eq!(after.len(), 2);
    Ok(())
}

#[test]
fn value_debounce_rapid_typing_keeps_first_and_latest_state()
-> Result<(), Box<dyn std::error::Error>> {
    let id = ElementId::parse("0x1234:00000003")?;
    let before: Vec<_> = "abcdef"
        .chars()
        .enumerate()
        .map(|(index, character)| {
            let mut item = event(
                u64::try_from(index).unwrap_or(u64::MAX),
                u64::try_from(index * 20).unwrap_or(u64::MAX),
                Some(id.clone()),
                AccessibleEventKind::ValueChanged,
            );
            item.value = Some(character.to_string());
            item
        })
        .collect();
    println!("readback=debounced_events edge=rapid_typing before={before:?}");
    let after = debounce_value_changes(before, Duration::from_millis(200));
    println!("readback=debounced_events edge=rapid_typing after={after:?}");
    assert!(after.len() <= 2);
    assert_eq!(
        after.last().and_then(|item| item.value.as_deref()),
        Some("f")
    );
    Ok(())
}

#[test]
fn value_debounce_focus_loss_flushes_pending_state() -> Result<(), Box<dyn std::error::Error>> {
    let id = ElementId::parse("0x1234:00000004")?;
    let mut first = event(1, 0, Some(id.clone()), AccessibleEventKind::ValueChanged);
    first.value = Some("a".to_owned());
    let mut second = event(2, 20, Some(id.clone()), AccessibleEventKind::ValueChanged);
    second.value = Some("b".to_owned());
    let focus = event(3, 25, Some(id), AccessibleEventKind::FocusChanged);
    let before = vec![first, second, focus];
    println!("readback=debounced_events edge=focus_loss before={before:?}");
    let after = debounce_value_changes(before, Duration::from_millis(200));
    println!("readback=debounced_events edge=focus_loss after={after:?}");
    assert_eq!(after.len(), 3);
    assert_eq!(after[1].value.as_deref(), Some("b"));
    assert_eq!(after[2].kind, AccessibleEventKind::FocusChanged);
    Ok(())
}

#[test]
fn runtime_id_hex_round_trips_through_composite_element_id()
-> Result<(), Box<dyn std::error::Error>> {
    let runtime = [42, -1, 0x1234_abcd_u32.cast_signed()];
    let runtime_hex = runtime_id_hex(&runtime);
    let id = element_id(0x12ab, &runtime_hex);
    println!("readback=element_id edge=runtime_hex before={runtime:?}");
    println!("readback=element_id edge=runtime_hex after={id}");
    let parts = id.parts()?;
    assert_eq!(parts.hwnd, 0x12ab);
    assert_eq!(parts.runtime_id_hex, "0000002affffffff1234abcd");
    Ok(())
}

#[test]
fn non_windows_uia_reports_not_available() {
    #[cfg(not(windows))]
    {
        let before = "focused_window";
        println!("readback=a11y_error edge=non_windows before={before}");
        let after = focused_window();
        println!("readback=a11y_error edge=non_windows after={after:?}");
        assert_eq!(
            after.err().map(|err| err.code()),
            Some(error_codes::A11Y_NOT_AVAILABLE)
        );
    }
}

#[tokio::test]
async fn cdp_probe_non_chromium_is_explicitly_not_chromium() {
    let before = ("notepad.exe", Vec::<u16>::new());
    println!("readback=cdp_diagnostics edge=non_chromium before={before:?}");
    let after = probe_chromium_cdp("notepad.exe", &[], Duration::from_millis(10)).await;
    println!("readback=cdp_diagnostics edge=non_chromium after={after:?}");
    assert_eq!(after.status, CdpStatus::NotChromium);
    assert!(after.reason_code.is_none());
}

#[tokio::test]
async fn cdp_probe_chromium_without_port_surfaces_unreachable_code() {
    let before = ("chrome.exe", Vec::<u16>::new());
    println!("readback=cdp_diagnostics edge=no_debug_port before={before:?}");
    let after = probe_chromium_cdp("chrome.exe", &[], Duration::from_millis(10)).await;
    println!("readback=cdp_diagnostics edge=no_debug_port after={after:?}");
    assert_eq!(after.status, CdpStatus::Unreachable);
    assert_eq!(
        after.reason_code.as_deref(),
        Some(error_codes::A11Y_CDP_UNREACHABLE)
    );
}

#[tokio::test]
async fn cdp_probe_reachable_debug_port_surfaces_capabilities()
-> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    println!("readback=cdp_diagnostics edge=reachable_port before=port:{port}");
    let after = probe_chromium_cdp("msedge.exe", &[port], Duration::from_secs(1)).await;
    println!("readback=cdp_diagnostics edge=reachable_port after={after:?}");
    assert_eq!(after.status, CdpStatus::Ok);
    assert_eq!(
        after.endpoint.as_deref(),
        Some(format!("http://127.0.0.1:{port}").as_str())
    );
    assert_eq!(after.capabilities, cdp_capabilities());
    Ok(())
}

proptest! {
    #[test]
    fn coalescing_never_outputs_same_key_inside_window(times in proptest::collection::vec(0_u64..500, 1..80)) {
        let id = element_id(0x1234, "00000005");
        let mut sorted = times;
        sorted.sort_unstable();
        let input: Vec<_> = sorted
            .iter()
            .enumerate()
            .map(|(index, at_ms)| event(u64::try_from(index).unwrap_or(u64::MAX), *at_ms, Some(id.clone()), AccessibleEventKind::NameChanged))
            .collect();
        let output = coalesce_events(input, Duration::from_millis(50));
        for pair in output.windows(2) {
            prop_assert!(
                pair[1].at_ms.saturating_sub(pair[0].at_ms) >= 50,
                "readback=coalesced_events edge=proptest after={output:?}"
            );
        }
    }
}

#[cfg(windows)]
#[test]
fn windows_win_event_hook_apartment_readback_is_sta() -> Result<(), Box<dyn std::error::Error>> {
    let (sender, _receiver) = tokio::sync::mpsc::unbounded_channel();
    println!("readback=winevent_hook edge=apartment before=unsubscribed");
    let subscription = subscribe_win_events(sender)?;
    let readback = subscription.readback().clone();
    println!("readback=winevent_hook edge=apartment after={readback:?}");
    assert!(readback.apartment.is_sta_family());
    assert_eq!(readback.hook_count, 10);
    assert_eq!(readback.event_ids.len(), 10);
    drop(subscription);
    Ok(())
}

#[cfg(windows)]
#[test]
fn windows_foreground_snapshot_round_trips_element_id() -> Result<(), Box<dyn std::error::Error>> {
    let root = focused_window()?;
    println!("readback=uia_snapshot edge=depth2 before=focused_window_resolved");
    let tree = snapshot(&root, 2)?;
    println!(
        "readback=uia_snapshot edge=depth2 after=root:{} nodes:{} max_depth:{}",
        tree.root,
        tree.nodes.len(),
        tree.max_depth
    );
    assert!(!tree.nodes.is_empty());
    let resolved = re_resolve(&tree.root)?;
    let round_trip = snapshot(&resolved, 0)?;
    println!(
        "readback=uia_snapshot edge=round_trip after=root:{} nodes:{}",
        round_trip.root,
        round_trip.nodes.len()
    );
    assert_eq!(round_trip.root, tree.root);
    Ok(())
}
