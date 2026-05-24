use synapse_core::{Action, Backend, Key, KeyCode, error_codes};
use tokio::time::{self, Duration, Instant};

use super::{ActionEmitter, HELD_KEY_MAX_DURATION_MS};

#[derive(Debug)]
pub(super) struct HeldKeyAutoRelease {
    key: Key,
    timer_id: u64,
}

impl ActionEmitter {
    pub(super) fn schedule_held_key_auto_release(&mut self, key: Key) {
        self.cancel_held_key_timer(&key);

        let timer_id = self.next_held_key_timer_id;
        self.next_held_key_timer_id = self.next_held_key_timer_id.wrapping_add(1);
        let deadline = Instant::now() + Duration::from_millis(HELD_KEY_MAX_DURATION_MS);
        let tx = self.auto_release_tx.clone();
        let timer_key = key.clone();
        let handle = tokio::spawn(async move {
            time::sleep_until(deadline).await;
            let _send_result = tx
                .send(HeldKeyAutoRelease {
                    key: timer_key,
                    timer_id,
                })
                .await;
        });

        self.held_key_timer_ids.insert(key.clone(), timer_id);
        self.held_key_timers.insert(key, handle);
    }

    pub(super) fn cancel_held_key_timer(&mut self, key: &Key) -> bool {
        self.held_key_timer_ids.remove(key);
        self.held_key_timers.remove(key).is_some_and(|handle| {
            handle.abort();
            true
        })
    }

    pub(super) fn abort_all_held_key_timers(&mut self) -> usize {
        let cancelled = self.held_key_timers.len();
        for (_key, handle) in self.held_key_timers.drain() {
            handle.abort();
        }
        self.held_key_timer_ids.clear();
        cancelled
    }

    pub(super) fn auto_release_held_key(
        &mut self,
        auto_release: &HeldKeyAutoRelease,
    ) -> Option<Action> {
        if self
            .held_key_timer_ids
            .get(&auto_release.key)
            .is_none_or(|timer_id| *timer_id != auto_release.timer_id)
        {
            return None;
        }

        self.held_key_timer_ids.remove(&auto_release.key);
        self.held_key_timers.remove(&auto_release.key);
        if !self.state.is_key_held(&auto_release.key) {
            return None;
        }

        self.state.release_key(&auto_release.key);
        tracing::warn!(
            code = %error_codes::STUCK_KEY_AUTO_RELEASED,
            held_ms = HELD_KEY_MAX_DURATION_MS,
            key = %key_log_label(&auto_release.key),
            key_debug = ?auto_release.key,
            "stuck key auto-released"
        );
        Some(Action::KeyUp {
            key: auto_release.key.clone(),
            backend: Backend::Auto,
        })
    }

    pub(super) fn held_key_timer_keys(&self) -> Vec<Key> {
        let mut keys: Vec<_> = self.held_key_timers.keys().cloned().collect();
        keys.sort_by_key(|key| format!("{key:?}"));
        keys
    }
}
pub(super) fn key_log_label(key: &Key) -> String {
    match &key.code {
        KeyCode::Named { value } => value.clone(),
        KeyCode::Symbol { value } => value.to_string(),
        KeyCode::HidCode { value } => format!("hid:{value}"),
    }
}
