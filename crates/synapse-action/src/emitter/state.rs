use std::collections::HashMap;

use bit_set::BitSet;
use synapse_core::{ButtonAction, GamepadReport, Key, MouseButton, PadId};
use tokio::sync::{mpsc, oneshot};

use crate::ActionResult;

pub type ActionSnapshotMessage = oneshot::Sender<ActionStateSnapshot>;

#[derive(Clone, Debug, PartialEq)]
pub struct ActionStateSnapshot {
    pub held_keys: Vec<Key>,
    pub held_key_bits: Vec<usize>,
    pub held_key_timer_keys: Vec<Key>,
    pub held_key_timer_count: usize,
    pub held_buttons: Vec<MouseButton>,
    pub held_button_bits: Vec<usize>,
    pub pad_state: HashMap<PadId, GamepadReport>,
}

#[derive(Debug)]
pub struct EmitState {
    pub(crate) held_keys: BitSet,
    pub(crate) held_buttons: BitSet,
    pub(crate) key_indices: HashMap<Key, usize>,
    pub(crate) keys_by_index: Vec<Key>,
    pub(crate) pad_state: HashMap<PadId, GamepadReport>,
}

impl EmitState {
    #[must_use]
    #[tracing::instrument(skip_all, fields(action_kind = "emit_state_new"))]
    pub fn new() -> Self {
        Self {
            held_keys: BitSet::new(),
            held_buttons: BitSet::new(),
            key_indices: HashMap::new(),
            keys_by_index: Vec::new(),
            pad_state: HashMap::new(),
        }
    }

    #[must_use]
    #[tracing::instrument(skip_all, fields(action_kind = "emit_state_snapshot"))]
    pub fn snapshot(&self) -> ActionStateSnapshot {
        ActionStateSnapshot {
            held_keys: self.held_keys(),
            held_key_bits: self.held_keys.iter().collect(),
            held_key_timer_keys: Vec::new(),
            held_key_timer_count: 0,
            held_buttons: self.held_buttons(),
            held_button_bits: self.held_buttons.iter().collect(),
            pad_state: self.pad_state.clone(),
        }
    }

    fn held_keys(&self) -> Vec<Key> {
        self.held_keys
            .iter()
            .filter_map(|index| self.keys_by_index.get(index).cloned())
            .collect()
    }

    fn held_buttons(&self) -> Vec<MouseButton> {
        self.held_buttons
            .iter()
            .filter_map(mouse_button_from_index)
            .collect()
    }

    pub(crate) fn release_all(&mut self) -> (usize, usize, usize) {
        let released_keys = self.held_keys.count();
        let released_buttons = self.held_buttons.count();
        let released_pads = self.pad_state.len();
        self.held_keys.make_empty();
        self.held_buttons.make_empty();
        self.pad_state.clear();
        (released_keys, released_buttons, released_pads)
    }

    pub(crate) fn hold_key(&mut self, key: &Key) {
        let index = self.key_index(key);
        self.held_keys.insert(index);
    }

    pub(crate) fn release_key(&mut self, key: &Key) {
        if let Some(index) = self.key_indices.get(key) {
            self.held_keys.remove(*index);
        }
    }

    pub(crate) fn is_key_held(&self, key: &Key) -> bool {
        self.key_indices
            .get(key)
            .is_some_and(|index| self.held_keys.contains(*index))
    }

    pub(crate) fn apply_mouse_button(&mut self, button: MouseButton, action: ButtonAction) {
        let index = mouse_button_index(button);
        match action {
            ButtonAction::Down => {
                self.held_buttons.insert(index);
            }
            ButtonAction::Up | ButtonAction::Press => {
                self.held_buttons.remove(index);
            }
        }
    }

    fn key_index(&mut self, key: &Key) -> usize {
        if let Some(index) = self.key_indices.get(key) {
            return *index;
        }
        let index = self.keys_by_index.len();
        self.keys_by_index.push(key.clone());
        self.key_indices.insert(key.clone(), index);
        index
    }
}

impl Default for EmitState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub struct ActionEmitterSnapshotHandle {
    tx: mpsc::Sender<ActionSnapshotMessage>,
}

impl ActionEmitterSnapshotHandle {
    #[must_use]
    #[tracing::instrument(skip_all, fields(action_kind = "snapshot_handle_new"))]
    pub fn new(tx: mpsc::Sender<ActionSnapshotMessage>) -> Self {
        Self { tx }
    }

    /// Reads the emitter's held-state snapshot through the actor task.
    ///
    /// # Errors
    ///
    /// Returns `ACTION_BACKEND_UNAVAILABLE` when the snapshot request or
    /// response channel is closed.
    #[tracing::instrument(skip_all, fields(action_kind = "snapshot"))]
    pub async fn snapshot(&self) -> ActionResult<ActionStateSnapshot> {
        let (snapshot_tx, snapshot_rx) = oneshot::channel();
        self.tx
            .send(snapshot_tx)
            .await
            .map_err(|_err| crate::ActionError::BackendUnavailable {
                detail: "action emitter snapshot channel is closed".to_owned(),
            })?;
        snapshot_rx
            .await
            .map_err(|_err| crate::ActionError::BackendUnavailable {
                detail: "action emitter dropped snapshot response".to_owned(),
            })
    }
}

/// Snapshot of the three production backends the actor dispatches through.
///
/// Resolved per-action via [`resolve_backend`]. The actor itself stays the
const fn mouse_button_index(button: MouseButton) -> usize {
    match button {
        MouseButton::Left => 0,
        MouseButton::Right => 1,
        MouseButton::Middle => 2,
        MouseButton::X1 => 3,
        MouseButton::X2 => 4,
    }
}

const fn mouse_button_from_index(index: usize) -> Option<MouseButton> {
    match index {
        0 => Some(MouseButton::Left),
        1 => Some(MouseButton::Right),
        2 => Some(MouseButton::Middle),
        3 => Some(MouseButton::X1),
        4 => Some(MouseButton::X2),
        _ => None,
    }
}
