use std::sync::Arc;

use synapse_core::{Action, ComboInput, error_codes};

use super::routing::{action_consumes_rate_limit, action_kind, resolved_backend_for_action};
use super::{ActionEmitter, EmitState};
use crate::{
    ActionBackend, ActionError, ActionResult, ResolvedBackend,
    rate_limit::retry_after_ms_for_snapshot,
};

impl ActionEmitter {
    #[tracing::instrument(skip_all, fields(action_kind = %action_kind(&action)))]
    pub(super) async fn execute(&mut self, action: Action) -> ActionResult<()> {
        crate::validate_action(&action)?;

        if matches!(action, Action::ReleaseAll) {
            return self.do_release_all("tool_invocation").await;
        }

        if action_consumes_rate_limit(&action) {
            let backend = resolved_backend_for_action(&action)?;
            self.consume_rate_limit(backend)?;
        }

        self.cancel_timers_for_release_actions(&action);

        let resolved = resolved_backend_for_action(&action)?;
        let backend = self.backends.pick(resolved);
        let result = self.dispatch_via_backend(backend, action.clone()).await;

        if result.is_ok() {
            self.schedule_timers_for_held_keys(&action);
        }

        result
    }

    async fn dispatch_via_backend(
        &mut self,
        backend: Arc<dyn ActionBackend>,
        action: Action,
    ) -> ActionResult<()> {
        let mut state = std::mem::take(&mut self.state);
        let task = tokio::task::spawn_blocking(move || {
            let result = backend.execute(&action, &mut state);
            (result, state)
        });
        match task.await {
            Ok((result, state)) => {
                self.state = state;
                result
            }
            Err(join_error) => {
                // The blocking task panicked or was cancelled; we lost the
                // moved EmitState. Fail-closed: surface the cause and reset
                // held-state to empty so the next action starts clean.
                self.state = EmitState::new();
                Err(ActionError::BackendUnavailable {
                    detail: format!(
                        "code={} backend.execute spawn_blocking join failed: {join_error}",
                        error_codes::ACTION_BACKEND_UNAVAILABLE
                    ),
                })
            }
        }
    }

    /// Cancels in-flight auto-release timers for any key the action is about
    /// to release. Done before backend dispatch so a timer cannot fire mid-call
    /// and enqueue a duplicate `KeyUp` against a key the backend has already
    /// released.
    fn cancel_timers_for_release_actions(&mut self, action: &Action) {
        match action {
            Action::KeyPress { key, .. } | Action::KeyUp { key, .. } => {
                self.cancel_held_key_timer(key);
            }
            Action::KeyChord { keys, .. } => {
                for key in keys {
                    self.cancel_held_key_timer(key);
                }
            }
            Action::Combo { steps, .. } => {
                for step in steps {
                    if let ComboInput::KeyUp { key } | ComboInput::KeyPress { key, .. } =
                        &step.input
                    {
                        self.cancel_held_key_timer(key);
                    }
                }
            }
            _ => {}
        }
    }

    /// Schedules safety auto-release timers for any key the action just put
    /// into a held-down state. Called only when the backend dispatch
    /// succeeded — and only for keys whose post-dispatch state actually
    /// shows them held. A `Combo` like
    /// `[KeyDown(ctrl), KeyUp(ctrl)]` nets to zero held keys, so no
    /// timer is scheduled even though the combo contained a `KeyDown` step.
    fn schedule_timers_for_held_keys(&mut self, action: &Action) {
        match action {
            Action::KeyDown { key, .. } if self.state.is_key_held(key) => {
                self.schedule_held_key_auto_release(key.clone());
            }
            Action::Combo { steps, .. } => {
                for step in steps {
                    if let ComboInput::KeyDown { key } = &step.input
                        && self.state.is_key_held(key)
                    {
                        self.schedule_held_key_auto_release(key.clone());
                    }
                }
            }
            _ => {}
        }
    }

    fn consume_rate_limit(&self, backend: ResolvedBackend) -> ActionResult<()> {
        let bucket = self.rate_limits.bucket(backend);
        if bucket.try_consume(1) {
            return Ok(());
        }

        let snapshot = bucket.snapshot();
        let retry_after_ms = retry_after_ms_for_snapshot(snapshot, 1);
        Err(ActionError::RateLimited {
            detail: format!(
                "backend={} retry_after_ms={} requested_tokens=1 available_tokens={} refill_rate_per_s={}",
                backend.as_str(),
                retry_after_ms,
                snapshot.tokens,
                snapshot.refill_rate_per_s
            ),
            retry_after_ms,
        })
    }

    #[tracing::instrument(skip_all, fields(action_kind = "release_all"))]
    pub(super) async fn release_all(&mut self, reason: &'static str) {
        let _release_result = self.do_release_all(reason).await;
    }

    /// Drives a `ReleaseAll` through the resolved backend (so the production
    /// path actually emits `SendInput` `KeyUp`s for every held key/button),
    /// aborts in-flight auto-release timers, and logs the drained snapshot.
    /// The blocking backend work runs on `spawn_blocking` so the runtime
    /// stays responsive.
    async fn do_release_all(&mut self, reason: &'static str) -> ActionResult<()> {
        let before = self.snapshot();
        let mut held_pad_ids: Vec<_> = before.pad_state.keys().copied().collect();
        held_pad_ids.sort_unstable();
        let released_keys = before.held_keys.len();
        let released_buttons = before.held_buttons.len();
        let released_pads = before.pad_state.len();

        let cancelled_key_timers = self.abort_all_held_key_timers();

        let resolved = resolved_backend_for_action(&Action::ReleaseAll)?;
        let backend = self.backends.pick(resolved);
        let result = self
            .dispatch_via_backend(Arc::clone(&backend), Action::ReleaseAll)
            .await;
        let result = if released_pads == 0 {
            result
        } else if let Some(vigem_backend) = self.backends.pick_vigem_if_distinct_from(&backend) {
            match (
                result,
                self.dispatch_via_backend(vigem_backend, Action::ReleaseAll)
                    .await,
            ) {
                (Ok(()), vigem_result) => vigem_result,
                (software_result @ Err(_), Ok(())) => software_result,
                (software_result @ Err(_), Err(_vigem_error)) => software_result,
            }
        } else {
            result
        };

        tracing::warn!(
            code = error_codes::SAFETY_RELEASE_ALL_FIRED,
            reason,
            held_keys = ?before.held_keys,
            held_key_bits = ?before.held_key_bits,
            held_key_timer_keys = ?before.held_key_timer_keys,
            held_key_timer_count = before.held_key_timer_count,
            held_buttons = ?before.held_buttons,
            held_button_bits = ?before.held_button_bits,
            held_pad_ids = ?held_pad_ids,
            released_keys,
            released_buttons,
            released_pads,
            cancelled_key_timers,
            backend_ok = result.is_ok(),
            "release_all drained action emitter held state"
        );

        // If the backend failed mid-release, clear the held bitmaps anyway —
        // the actor's state must reflect what the safety path attempted, so
        // the next ReleaseAll snapshot does not loop on the same keys.
        if result.is_err() {
            self.state.release_all();
        }
        result
    }
}
