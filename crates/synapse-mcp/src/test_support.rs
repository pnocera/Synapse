use std::sync::{Mutex, MutexGuard, PoisonError};

static LEASE_SERIAL: Mutex<()> = Mutex::new(());

pub(crate) fn lease_serial(reason: &str) -> MutexGuard<'static, ()> {
    let guard = LEASE_SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    let _prior = synapse_action::lease::force_clear(reason);
    guard
}

pub(crate) fn reset_lease(reason: &str) {
    let _prior = synapse_action::lease::force_clear(reason);
}
