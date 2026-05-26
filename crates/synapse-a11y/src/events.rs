use std::{collections::HashMap, time::Duration};

use serde::{Deserialize, Serialize};
use synapse_core::ElementId;
use tokio::sync::mpsc::UnboundedSender;

use crate::{A11yResult, platform};

pub type AccessibleEventSender = UnboundedSender<AccessibleEvent>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessibleEvent {
    pub seq: u64,
    pub at_ms: u64,
    pub window_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<ElementId>,
    pub kind: AccessibleEventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessibleEventKind {
    ForegroundChanged,
    FocusChanged,
    ValueChanged,
    NameChanged,
    ElementAppeared,
    ElementDisappeared,
    SelectionChanged,
    MenuStart,
    MenuEnd,
    Alert,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WinEventHookReadback {
    pub thread_id: u32,
    pub apartment: ComApartmentKind,
    pub hook_count: usize,
    pub event_ids: Vec<u32>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComApartmentKind {
    Sta,
    Mta,
    Neutral,
    MainSta,
    Unknown,
    Unsupported,
}

impl ComApartmentKind {
    #[must_use]
    pub const fn is_sta_family(self) -> bool {
        matches!(self, Self::Sta | Self::MainSta)
    }
}

pub struct WinEventSubscription {
    inner: platform::WinEventSubscription,
}

impl WinEventSubscription {
    #[allow(clippy::missing_const_for_fn)]
    #[must_use]
    pub fn readback(&self) -> &WinEventHookReadback {
        self.inner.readback()
    }
}

#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
struct EventKey {
    window_id: i64,
    kind: AccessibleEventKind,
    element_id_hash: u64,
}

impl EventKey {
    fn from_event(event: &AccessibleEvent) -> Self {
        use std::{
            collections::hash_map::DefaultHasher,
            hash::{Hash, Hasher},
        };

        let mut hasher = DefaultHasher::new();
        event.element_id.hash(&mut hasher);
        Self {
            window_id: event.window_id,
            kind: event.kind,
            element_id_hash: hasher.finish(),
        }
    }
}

#[must_use]
pub fn coalesce_events<I>(events: I, window: Duration) -> Vec<AccessibleEvent>
where
    I: IntoIterator<Item = AccessibleEvent>,
{
    let window_ms = u64::try_from(window.as_millis()).unwrap_or(u64::MAX);
    let mut output = Vec::new();
    let mut pending: Option<AccessibleEvent> = None;

    for event in events {
        let Some(current) = pending.take() else {
            pending = Some(event);
            continue;
        };

        let same_key = EventKey::from_event(&current) == EventKey::from_event(&event);
        let within_window = event.at_ms.saturating_sub(current.at_ms) < window_ms;

        if !(same_key && within_window) {
            output.push(current);
        }
        pending = Some(event);
    }

    if let Some(event) = pending {
        output.push(event);
    }

    output
}

#[must_use]
pub fn debounce_value_changes<I>(events: I, window: Duration) -> Vec<AccessibleEvent>
where
    I: IntoIterator<Item = AccessibleEvent>,
{
    let window_ms = u64::try_from(window.as_millis()).unwrap_or(u64::MAX);
    let mut output = Vec::new();
    let mut last_emitted = HashMap::<EventKey, u64>::new();
    let mut pending = HashMap::<EventKey, AccessibleEvent>::new();

    for event in events {
        if event.kind == AccessibleEventKind::FocusChanged {
            flush_pending(&mut pending, &mut output);
            output.push(event);
            continue;
        }

        if event.kind != AccessibleEventKind::ValueChanged {
            output.push(event);
            continue;
        }

        let key = EventKey::from_event(&event);
        match last_emitted.get(&key).copied() {
            Some(last_at) if event.at_ms.saturating_sub(last_at) < window_ms => {
                pending.insert(key, event);
            }
            _ => {
                pending.remove(&key);
                last_emitted.insert(key, event.at_ms);
                output.push(event);
            }
        }
    }

    flush_pending(&mut pending, &mut output);
    output.sort_by_key(|event| (event.at_ms, event.seq));
    output
}

fn flush_pending(
    pending: &mut HashMap<EventKey, AccessibleEvent>,
    output: &mut Vec<AccessibleEvent>,
) {
    output.extend(pending.drain().map(|(_key, event)| event));
}

/// Starts the dedicated `WinEvent` hook thread and marshals events into `sender`.
///
/// # Errors
///
/// Returns a structured UIA error when the hook thread cannot initialize, or
/// `A11Y_NOT_AVAILABLE` on non-Windows platforms.
pub fn subscribe_win_events(sender: AccessibleEventSender) -> A11yResult<WinEventSubscription> {
    let inner = platform::subscribe_win_events(sender)?;
    Ok(WinEventSubscription { inner })
}
