use std::{
    panic::{self, AssertUnwindSafe},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use synapse_action::{ActionHandle, RELEASE_ALL_HANDLE, install_panic_hook};
use synapse_core::Action;

#[test]
fn panic_hook_timeout_keeps_release_all_in_queue_and_preserves_previous_hook() {
    let (handle, mut action_rx) = ActionHandle::channel();
    assert!(
        RELEASE_ALL_HANDLE.set(handle).is_ok(),
        "RELEASE_ALL_HANDLE should be unset at integration-test process start"
    );

    let previous_count = Arc::new(AtomicUsize::new(0));
    let previous_count_for_hook = Arc::clone(&previous_count);
    panic::set_hook(Box::new(move |_info| {
        previous_count_for_hook.fetch_add(1, Ordering::SeqCst);
    }));

    println!(
        "source_of_truth=panic_hook_queue edge=timeout before=queued:{} previous_count:{}",
        action_rx.len(),
        previous_count.load(Ordering::SeqCst)
    );

    install_panic_hook();
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        panic!("synthetic #179 timeout panic");
    }));
    assert!(result.is_err());
    assert_eq!(previous_count.load(Ordering::SeqCst), 1);
    assert_eq!(action_rx.len(), 1);

    let action_label = match action_rx.try_recv() {
        Ok((Action::ReleaseAll, _ack)) => "release_all",
        Ok((_action, _ack)) => "unexpected",
        Err(error) => panic!("release_all action should remain queued after timeout: {error:?}"),
    };
    assert_eq!(action_label, "release_all");

    println!(
        "source_of_truth=panic_hook_queue edge=timeout after_queued_before_drain:1 after_drained_action:{action_label} previous_count:{} panic_caught:{}",
        previous_count.load(Ordering::SeqCst),
        result.is_err()
    );
}
