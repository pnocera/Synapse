use std::{
    collections::BTreeMap,
    num::NonZeroUsize,
    sync::{Arc, Mutex},
};

use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use synapse_core::{Event, EventFilter};
use synapse_reflex::{EventBus, EventBusError, PublishReport};

mod lossy;
mod replay;
mod ring;
mod stream;

use ring::Subscription;

const LAST_EVENT_ID: &str = "Last-Event-ID";
const SUBSCRIPTION_ID_HEADER: &str = "Synapse-Subscription-Id";
const MANUAL_ENV: &str = "SYNAPSE_HTTP_SSE_MANUAL";

#[derive(Clone, Debug)]
pub struct SseState {
    inner: Arc<SseStateInner>,
}

#[derive(Debug)]
struct SseStateInner {
    event_bus: EventBus,
    subscriptions: Mutex<BTreeMap<String, Arc<Subscription>>>,
    manual_routes_enabled: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EventsQuery {
    pub subscription_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StatsQuery {
    pub subscription_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PublishRequest {
    pub events: Vec<Event>,
}

#[derive(Clone, Debug, Serialize)]
struct PublishResponse {
    matched: usize,
    queued: usize,
    dropped: u64,
    subscriptions_synced: usize,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum SseOpenError {
    BadLastEventId,
    SubscribeUnavailable(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SseSubscribeError {
    CapReached { limit: usize },
    FilterInvalid { detail: String },
    StateUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SseCancelError {
    NotFound,
    StateUnavailable,
}

impl SseState {
    #[cfg(test)]
    pub(crate) fn from_env() -> Self {
        Self::with_max_subscriptions(synapse_reflex::DEFAULT_MAX_SUBSCRIPTIONS_NONZERO)
    }

    pub(crate) fn with_max_subscriptions(max_subscriptions: NonZeroUsize) -> Self {
        Self {
            inner: Arc::new(SseStateInner {
                event_bus: EventBus::with_max_subscriptions(max_subscriptions),
                subscriptions: Mutex::new(BTreeMap::new()),
                manual_routes_enabled: manual_routes_enabled(),
            }),
        }
    }

    pub(super) fn open(&self, headers: &HeaderMap, query: EventsQuery) -> Response {
        let last_event_id = match replay::parse_last_event_id(headers) {
            Ok(value) => value,
            Err(error) => return error.into_response(),
        };
        let subscription = match self.subscription_for(query.subscription_id, last_event_id) {
            Ok(subscription) => subscription,
            Err(error) => return error.into_response(),
        };
        let frames = replay::frames_after(&subscription, last_event_id);
        stream::sse_response(subscription, frames, last_event_id)
    }

    pub(crate) fn subscribe(
        &self,
        filter: EventFilter,
        kinds: Vec<String>,
        snapshot_first: bool,
    ) -> Result<String, SseSubscribeError> {
        self.create_subscription_with(filter, kinds, snapshot_first)
            .map(|subscription| subscription.id().to_owned())
    }

    pub(crate) fn event_bus(&self) -> EventBus {
        self.inner.event_bus.clone()
    }

    pub(crate) fn active_subscription_count(&self) -> usize {
        self.inner
            .subscriptions
            .lock()
            .map_or(0, |subscriptions| subscriptions.len())
    }

    pub(crate) fn cancel(&self, id: &str) -> Result<(), SseCancelError> {
        let removed_from_map = {
            let mut subscriptions = self
                .inner
                .subscriptions
                .lock()
                .map_err(|_| SseCancelError::StateUnavailable)?;
            subscriptions.remove(id).is_some()
        };
        let removed_from_bus = self.inner.event_bus.unsubscribe(id);
        if removed_from_map || removed_from_bus {
            Ok(())
        } else {
            Err(SseCancelError::NotFound)
        }
    }

    pub(super) fn publish(&self, request: PublishRequest) -> Response {
        if !self.inner.manual_routes_enabled {
            return StatusCode::NOT_FOUND.into_response();
        }
        let report = self.publish_events(request.events);
        let subscriptions_synced = self.sync_all();
        axum::Json(PublishResponse {
            matched: report.matched,
            queued: report.queued,
            dropped: report.dropped,
            subscriptions_synced,
        })
        .into_response()
    }

    fn publish_events(&self, events: Vec<Event>) -> PublishReport {
        let mut total = PublishReport::default();
        // ADR-0007: the manual HTTP route accepts a JSON array for operator
        // convenience, but every item is still published as an individual event.
        for event in events {
            let report = self.inner.event_bus.publish(event);
            total.matched = total.matched.saturating_add(report.matched);
            total.queued = total.queued.saturating_add(report.queued);
            total.dropped = total.dropped.saturating_add(report.dropped);
        }
        total
    }

    pub(super) fn stats(&self, query: &StatsQuery) -> Response {
        if !self.inner.manual_routes_enabled {
            return StatusCode::NOT_FOUND.into_response();
        }
        let Some(subscription) = self.existing_subscription(&query.subscription_id) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        Self::sync_subscription(&subscription);
        axum::Json(subscription.stats()).into_response()
    }

    fn subscription_for(
        &self,
        subscription_id: Option<String>,
        last_event_id: Option<u64>,
    ) -> Result<Arc<Subscription>, SseOpenError> {
        if let Some(id) = subscription_id
            && let Some(subscription) = self.existing_subscription(&id)
        {
            Self::sync_subscription(&subscription);
            match last_event_id {
                None => return Ok(subscription),
                Some(last_id)
                    if subscription
                        .latest_seq()
                        .is_some_and(|latest_seq| last_id <= latest_seq) =>
                {
                    return Ok(subscription);
                }
                Some(_) => {}
            }
        }
        self.create_subscription()
    }

    fn existing_subscription(&self, id: &str) -> Option<Arc<Subscription>> {
        let subscriptions = self.inner.subscriptions.lock().ok()?;
        subscriptions.get(id).cloned()
    }

    fn create_subscription(&self) -> Result<Arc<Subscription>, SseOpenError> {
        self.create_subscription_with(EventFilter::All, Vec::new(), false)
            .map_err(|error| SseOpenError::SubscribeUnavailable(error.code()))
    }

    fn create_subscription_with(
        &self,
        filter: EventFilter,
        kinds: Vec<String>,
        snapshot_first: bool,
    ) -> Result<Arc<Subscription>, SseSubscribeError> {
        let handle = self
            .inner
            .event_bus
            .subscribe(filter, kinds, snapshot_first)
            .map_err(SseSubscribeError::from)?;
        let id = handle.id().to_owned();
        let subscription = Arc::new(Subscription::new(handle));
        {
            let mut subscriptions = self
                .inner
                .subscriptions
                .lock()
                .map_err(|_| SseSubscribeError::StateUnavailable)?;
            subscriptions.insert(id, Arc::clone(&subscription));
        }
        Ok(subscription)
    }

    fn sync_all(&self) -> usize {
        let subscriptions = self
            .inner
            .subscriptions
            .lock()
            .map(|items| items.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for subscription in &subscriptions {
            Self::sync_subscription(subscription);
        }
        subscriptions.len()
    }

    fn sync_subscription(subscription: &Subscription) {
        let events = subscription.handle.drain();
        if events.is_empty() {
            let bus_dropped = subscription.handle.take_dropped_since_read();
            if bus_dropped > 0 {
                subscription.record_dropped(bus_dropped);
            }
            if subscription.handle.take_lossy() {
                subscription.record_lossy();
            }
            return;
        }
        let bus_dropped = subscription.handle.take_dropped_since_read();
        if bus_dropped > 0 {
            subscription.record_dropped(bus_dropped);
        }
        if subscription.handle.take_lossy() {
            subscription.record_lossy();
        }
        subscription.push_events(events);
    }
}

impl SseOpenError {
    fn into_response(self) -> Response {
        match self {
            Self::BadLastEventId => {
                (StatusCode::BAD_REQUEST, "malformed Last-Event-ID").into_response()
            }
            Self::SubscribeUnavailable(code) => {
                (StatusCode::SERVICE_UNAVAILABLE, code).into_response()
            }
        }
    }
}

impl SseSubscribeError {
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::CapReached { .. } => synapse_core::error_codes::SUBSCRIPTION_CAP_REACHED,
            Self::FilterInvalid { .. } => synapse_core::error_codes::TOOL_PARAMS_INVALID,
            Self::StateUnavailable => synapse_core::error_codes::TOOL_INTERNAL_ERROR,
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::CapReached { limit } => {
                format!("subscription cap reached: limit {limit}")
            }
            Self::FilterInvalid { detail } => format!("event filter invalid: {detail}"),
            Self::StateUnavailable => "subscription state lock poisoned".to_owned(),
        }
    }
}

impl SseCancelError {
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::NotFound => synapse_core::error_codes::SUBSCRIPTION_NOT_FOUND,
            Self::StateUnavailable => synapse_core::error_codes::TOOL_INTERNAL_ERROR,
        }
    }

    pub(crate) fn message(&self, subscription_id: &str) -> String {
        match self {
            Self::NotFound => format!("subscription not found: {subscription_id}"),
            Self::StateUnavailable => "subscription state lock poisoned".to_owned(),
        }
    }
}

impl From<EventBusError> for SseSubscribeError {
    fn from(value: EventBusError) -> Self {
        match value {
            EventBusError::SubscriptionCapReached { limit } => Self::CapReached { limit },
            EventBusError::FilterInvalid { detail } => Self::FilterInvalid { detail },
        }
    }
}

fn manual_routes_enabled() -> bool {
    std::env::var(MANUAL_ENV).is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::match_wildcard_for_single_variants,
    clippy::similar_names,
    reason = "unit tests intentionally keep failure messages close to the assertion"
)]
mod tests;
