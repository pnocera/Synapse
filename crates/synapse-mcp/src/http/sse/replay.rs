use std::{collections::VecDeque, sync::Arc};

use axum::http::HeaderMap;

use super::{LAST_EVENT_ID, SseOpenError, ring::Subscription, stream::SseFrame};

pub(super) fn frames_after(
    subscription: &Arc<Subscription>,
    last_event_id: Option<u64>,
) -> VecDeque<SseFrame> {
    super::SseState::sync_subscription(subscription);
    let (events, gap_lossy) = subscription.events_after(last_event_id);
    if events.is_empty() {
        return VecDeque::new();
    }
    let pending_lossy = subscription.take_lossy_pending();
    let lossy = gap_lossy || pending_lossy;
    let mut frames = VecDeque::with_capacity(events.len() + usize::from(lossy));
    if lossy {
        frames.push_back(SseFrame::subscription_started(subscription.id(), true));
    }
    for (index, event) in events.into_iter().enumerate() {
        frames.push_back(SseFrame::event(
            subscription.id(),
            event,
            lossy && index == 0,
        ));
    }
    frames
}

pub(super) fn parse_last_event_id(headers: &HeaderMap) -> Result<Option<u64>, SseOpenError> {
    let Some(raw) = headers.get(LAST_EVENT_ID) else {
        return Ok(None);
    };
    let raw = raw
        .to_str()
        .map_err(|_| SseOpenError::BadLastEventId)?
        .trim();
    raw.parse::<u64>()
        .map(Some)
        .map_err(|_| SseOpenError::BadLastEventId)
}
