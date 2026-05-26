use chrono::Utc;
use synapse_core::{Event, EventFilter, EventSource};

use super::{
    EventsQuery, SseState, replay,
    ring::BufferedEvent,
    stream::{SseFrame, event_data},
};

fn event(seq: u64, kind: &str) -> Event {
    Event {
        seq,
        at: Utc::now(),
        source: EventSource::System,
        kind: kind.to_owned(),
        data: serde_json::json!({"value": seq}),
        correlations: Vec::new(),
    }
}

#[test]
fn event_frame_is_stable_for_known_input() {
    let event = event(7, "tick");
    let data = event_data("sub-1", 1, &event, true).to_string();
    assert!(data.contains("\"subscription_id\":\"sub-1\""));
    assert!(data.contains("\"stream_seq\":1"));
    assert!(data.contains("\"seq\":7"));
    assert!(data.contains("\"lossy\":true"));
    assert_eq!(
        SseFrame::event(
            "sub-1",
            BufferedEvent {
                stream_seq: 1,
                event,
            },
            true,
        )
        .seq(),
        Some(1)
    );
}

#[test]
fn state_creates_subscription_with_empty_initial_body() {
    let state = SseState::from_env();
    let response = state.open(&axum::http::HeaderMap::new(), EventsQuery::default());
    assert_eq!(response.status(), axum::http::StatusCode::OK);
}

#[test]
fn sparse_domain_seq_gets_contiguous_stream_seq_without_loss() {
    let state = SseState::from_env();
    let subscription = state
        .create_subscription_with(
            EventFilter::Kind {
                kind: "reflex_fired".to_owned(),
            },
            Vec::new(),
            false,
        )
        .expect("subscription should register");

    state.publish_events(vec![event(62_488, "reflex_fired")]);
    SseState::sync_subscription(&subscription);
    let stats = subscription.stats();
    assert_eq!(stats.ring_len, 1);
    assert_eq!(stats.oldest_seq, Some(1));
    assert_eq!(stats.latest_seq, Some(1));
    assert_eq!(stats.oldest_event_seq, Some(62_488));
    assert_eq!(stats.latest_event_seq, Some(62_488));
    assert_eq!(stats.dropped_total, 0);
    assert!(!stats.lossy_pending);

    let frames = replay::frames_after(&subscription, None);
    assert_eq!(frames.len(), 1);
    match frames.front().expect("one event frame") {
        SseFrame::Event {
            stream_seq,
            event,
            lossy,
            ..
        } => {
            assert_eq!(*stream_seq, 1);
            assert_eq!(event.seq, 62_488);
            assert!(!lossy);
        }
        other => panic!("expected event frame, got {other:?}"),
    }
}

#[test]
fn last_event_id_uses_stream_seq_not_domain_event_seq() {
    let state = SseState::from_env();
    let subscription = state
        .create_subscription_with(EventFilter::All, Vec::new(), false)
        .expect("subscription should register");

    state.publish_events(vec![event(10, "first"), event(1_000, "second")]);
    let frames = replay::frames_after(&subscription, Some(1));
    assert_eq!(frames.len(), 1);
    match frames.front().expect("second event frame") {
        SseFrame::Event {
            stream_seq,
            event,
            lossy,
            ..
        } => {
            assert_eq!(*stream_seq, 2);
            assert_eq!(event.seq, 1_000);
            assert!(!lossy);
        }
        other => panic!("expected event frame, got {other:?}"),
    }
}
