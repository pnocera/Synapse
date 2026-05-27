use std::{
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use synapse_core::{Action, Backend, ComboStep};
use tokio::sync::{mpsc, oneshot};

use crate::{ActionError, ActionResult, validate_action};

pub const ACTION_QUEUE_CAPACITY: usize = 256;

pub type ActionMessage = (Action, oneshot::Sender<ActionResult<()>>);

pub static RELEASE_ALL_HANDLE: OnceLock<ActionHandle> = OnceLock::new();

pub trait ActionComboScheduler: Send + Sync {
    /// Schedules combo steps through an external scheduler.
    ///
    /// # Errors
    ///
    /// Returns an [`ActionError`] when the scheduler is unavailable or rejects
    /// the combo.
    fn schedule_combo(&self, steps: Vec<ComboStep>, backend: Backend) -> ActionResult<()>;
}

#[derive(Clone)]
pub struct ActionHandle {
    tx: mpsc::Sender<ActionMessage>,
    combo_scheduler: Arc<Mutex<Option<Arc<dyn ActionComboScheduler>>>>,
}

impl std::fmt::Debug for ActionHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActionHandle")
            .finish_non_exhaustive()
    }
}

impl ActionHandle {
    #[must_use]
    pub fn new(tx: mpsc::Sender<ActionMessage>) -> Self {
        Self {
            tx,
            combo_scheduler: Arc::new(Mutex::new(None)),
        }
    }

    #[must_use]
    pub fn channel() -> (Self, mpsc::Receiver<ActionMessage>) {
        let (tx, rx) = mpsc::channel(ACTION_QUEUE_CAPACITY);
        (Self::new(tx), rx)
    }

    /// Installs the scheduler used to route [`Action::Combo`] through the
    /// reflex runtime instead of flattening it directly in the action emitter.
    ///
    /// # Errors
    ///
    /// Returns `ACTION_BACKEND_UNAVAILABLE` if the bridge slot is poisoned.
    pub fn install_combo_scheduler(
        &self,
        scheduler: Arc<dyn ActionComboScheduler>,
    ) -> ActionResult<()> {
        let mut combo_scheduler =
            self.combo_scheduler
                .lock()
                .map_err(|_err| ActionError::BackendUnavailable {
                    detail: "action combo scheduler bridge is poisoned".to_owned(),
                })?;
        *combo_scheduler = Some(scheduler);
        drop(combo_scheduler);
        Ok(())
    }

    /// Enqueues an action and waits for the emitter acknowledgement.
    ///
    /// # Errors
    ///
    /// Returns `ACTION_BACKEND_UNAVAILABLE` when the emitter channel or
    /// acknowledgement path is closed, or the emitter's own `ActionError`.
    pub async fn execute(&self, action: Action) -> ActionResult<()> {
        validate_action(&action)?;
        if let Action::Combo { steps, backend } = &action
            && let Some(scheduler) = self.combo_scheduler()?
        {
            return scheduler.schedule_combo(steps.clone(), *backend);
        }
        let (ack_tx, ack_rx) = oneshot::channel();
        self.tx
            .send((action, ack_tx))
            .await
            .map_err(|_err| ActionError::BackendUnavailable {
                detail: "action emitter channel is closed".to_owned(),
            })?;
        ack_rx
            .await
            .map_err(|_err| ActionError::BackendUnavailable {
                detail: "action emitter dropped acknowledgement".to_owned(),
            })?
    }

    /// Attempts to enqueue an action without waiting for emitter completion.
    ///
    /// # Errors
    ///
    /// Returns `ACTION_QUEUE_FULL` when the bounded queue is saturated, or
    /// `ACTION_BACKEND_UNAVAILABLE` when the emitter channel is closed.
    pub fn try_execute(&self, action: Action) -> ActionResult<()> {
        validate_action(&action)?;
        if let Action::Combo { steps, backend } = &action
            && let Some(scheduler) = self.combo_scheduler()?
        {
            return scheduler.schedule_combo(steps.clone(), *backend);
        }
        let (ack_tx, _ack_rx) = oneshot::channel();
        self.tx.try_send((action, ack_tx)).map_err(map_try_send)?;
        Ok(())
    }

    /// Enqueues `ReleaseAll` and synchronously waits for its acknowledgement.
    ///
    /// # Errors
    ///
    /// Returns `ACTION_QUEUE_FULL` if the bounded queue is saturated, or
    /// `ACTION_BACKEND_UNAVAILABLE` if the acknowledgement closes or times out.
    pub fn fire_release_all_blocking_with_timeout(&self, timeout: Duration) -> ActionResult<()> {
        let (ack_tx, mut ack_rx) = oneshot::channel();
        self.tx
            .try_send((Action::ReleaseAll, ack_tx))
            .map_err(map_try_send)?;

        let deadline = Instant::now() + timeout;
        loop {
            match ack_rx.try_recv() {
                Ok(result) => return result,
                Err(oneshot::error::TryRecvError::Closed) => {
                    return Err(ActionError::BackendUnavailable {
                        detail: "release_all acknowledgement channel closed".to_owned(),
                    });
                }
                Err(oneshot::error::TryRecvError::Empty) if Instant::now() >= deadline => {
                    return Err(ActionError::BackendUnavailable {
                        detail: format!("release_all acknowledgement timed out after {timeout:?}"),
                    });
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }

    fn combo_scheduler(&self) -> ActionResult<Option<Arc<dyn ActionComboScheduler>>> {
        self.combo_scheduler
            .lock()
            .map(|scheduler| scheduler.clone())
            .map_err(|_err| ActionError::BackendUnavailable {
                detail: "action combo scheduler bridge is poisoned".to_owned(),
            })
    }
}

fn map_try_send(error: mpsc::error::TrySendError<ActionMessage>) -> ActionError {
    match error {
        mpsc::error::TrySendError::Full(_message) => ActionError::QueueFull {
            detail: format!("action queue capacity {ACTION_QUEUE_CAPACITY} is full"),
        },
        mpsc::error::TrySendError::Closed(_message) => ActionError::BackendUnavailable {
            detail: "action emitter channel is closed".to_owned(),
        },
    }
}
