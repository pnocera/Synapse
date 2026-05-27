use std::sync::{Arc, Mutex, Weak};

use synapse_action::{ActionComboScheduler, ActionError, ActionHandle, ActionResult};
use synapse_core::{Backend, ComboStep, new_reflex_id};

use crate::{ComboParams, ReflexRuntime, ScheduledReflex};

struct ReflexComboScheduler {
    runtime: Weak<Mutex<ReflexRuntime>>,
}

impl ActionComboScheduler for ReflexComboScheduler {
    fn schedule_combo(&self, steps: Vec<ComboStep>, backend: Backend) -> ActionResult<()> {
        let runtime = self
            .runtime
            .upgrade()
            .ok_or_else(|| ActionError::BackendUnavailable {
                detail: "reflex runtime is unavailable for action combo scheduling".to_owned(),
            })?;
        let mut runtime = runtime
            .lock()
            .map_err(|_err| ActionError::BackendUnavailable {
                detail: "reflex runtime lock poisoned during action combo scheduling".to_owned(),
            })?;
        let reflex = ScheduledReflex::combo(new_reflex_id(), ComboParams::new(steps, backend));
        runtime
            .register(&reflex)
            .map_err(|error| ActionError::BackendUnavailable {
                detail: format!("reflex combo scheduling failed: {error}"),
            })?;
        drop(runtime);
        Ok(())
    }
}

/// Installs a bridge so `ActionHandle::execute(Action::Combo)` schedules a
/// one-shot combo reflex when a reflex runtime owns the handle.
///
/// # Errors
///
/// Returns `ACTION_BACKEND_UNAVAILABLE` when the runtime or handle bridge slot
/// is poisoned.
pub fn install_action_combo_scheduler(runtime: &Arc<Mutex<ReflexRuntime>>) -> ActionResult<()> {
    let action_handle = action_handle(runtime)?;
    action_handle.install_combo_scheduler(Arc::new(ReflexComboScheduler {
        runtime: Arc::downgrade(runtime),
    }))
}

fn action_handle(runtime: &Arc<Mutex<ReflexRuntime>>) -> ActionResult<ActionHandle> {
    runtime
        .lock()
        .map(|runtime| runtime.action_handle().clone())
        .map_err(|_err| ActionError::BackendUnavailable {
            detail: "reflex runtime lock poisoned during action combo bridge install".to_owned(),
        })
}
