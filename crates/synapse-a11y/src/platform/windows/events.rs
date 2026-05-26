use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use synapse_core::{ElementId, element_id};
use tokio::sync::mpsc::UnboundedSender;
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    System::Com::{
        APTTYPE, APTTYPE_MAINSTA, APTTYPE_MTA, APTTYPE_NA, APTTYPE_STA, APTTYPEQUALIFIER,
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoGetApartmentType, CoInitializeEx,
        CoUninitialize,
    },
    System::Threading::GetCurrentThreadId,
    UI::{
        Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
        WindowsAndMessaging::{
            DispatchMessageW, EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, EVENT_OBJECT_FOCUS,
            EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_SELECTION, EVENT_OBJECT_VALUECHANGE,
            EVENT_SYSTEM_ALERT, EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_MENUEND,
            EVENT_SYSTEM_MENUSTART, GetMessageW, MSG, PostThreadMessageW, TranslateMessage,
            WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP,
        },
    },
};

use crate::{
    A11yError, A11yResult, AccessibleEvent, AccessibleEventKind, ComApartmentKind,
    WinEventHookReadback,
};

use super::snapshot::invalidate_snapshot_cache;

static WIN_EVENT_SENDER: Mutex<Option<UnboundedSender<AccessibleEvent>>> = Mutex::new(None);

const WIN_EVENT_IDS: [u32; 10] = [
    EVENT_SYSTEM_FOREGROUND,
    EVENT_OBJECT_FOCUS,
    EVENT_OBJECT_VALUECHANGE,
    EVENT_OBJECT_NAMECHANGE,
    EVENT_OBJECT_CREATE,
    EVENT_OBJECT_DESTROY,
    EVENT_OBJECT_SELECTION,
    EVENT_SYSTEM_MENUSTART,
    EVENT_SYSTEM_MENUEND,
    EVENT_SYSTEM_ALERT,
];
pub struct WinEventSubscription {
    stop: Arc<AtomicBool>,
    thread_id: u32,
    join: Option<JoinHandle<()>>,
    readback: WinEventHookReadback,
}

impl WinEventSubscription {
    pub const fn readback(&self) -> &WinEventHookReadback {
        &self.readback
    }
}

impl Drop for WinEventSubscription {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_APP, WPARAM(0), LPARAM(0)) };
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        if let Ok(mut guard) = WIN_EVENT_SENDER.lock() {
            *guard = None;
        }
    }
}

pub fn subscribe_win_events(
    sender: UnboundedSender<AccessibleEvent>,
) -> A11yResult<WinEventSubscription> {
    {
        let mut guard = WIN_EVENT_SENDER
            .lock()
            .map_err(|err| A11yError::internal(err.to_string()))?;
        *guard = Some(sender);
    }

    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let (ready_tx, ready_rx) = mpsc::channel();
    let join = thread::Builder::new()
        .name("synapse-a11y-winevent".to_owned())
        .spawn(move || win_event_thread(thread_stop, ready_tx))
        .map_err(|err| A11yError::internal(err.to_string()))?;
    let readback = ready_rx
        .recv_timeout(Duration::from_secs(3))
        .map_err(|err| A11yError::internal(format!("WinEvent hook did not initialize: {err}")))??;

    Ok(WinEventSubscription {
        stop,
        thread_id: readback.thread_id,
        join: Some(join),
        readback,
    })
}
#[allow(clippy::needless_pass_by_value)]
fn win_event_thread(stop: Arc<AtomicBool>, ready: mpsc::Sender<A11yResult<WinEventHookReadback>>) {
    let thread_id = unsafe { GetCurrentThreadId() };
    let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
    if init.is_err() {
        let _ = ready.send(Err(A11yError::internal(format!("{init:?}"))));
        return;
    }

    let hooks = install_hooks();
    if hooks.is_empty() {
        let _ = ready.send(Err(A11yError::internal(
            "SetWinEventHook returned no hooks",
        )));
        unsafe {
            CoUninitialize();
        }
        return;
    }

    let readback = WinEventHookReadback {
        thread_id,
        apartment: read_current_apartment(),
        hook_count: hooks.len(),
        event_ids: WIN_EVENT_IDS.to_vec(),
    };
    let _ = ready.send(Ok(readback));

    let mut msg = MSG::default();
    while !stop.load(Ordering::SeqCst) {
        let result = unsafe { GetMessageW(&raw mut msg, None, 0, 0) };
        if result.0 <= 0 || msg.message == WM_APP {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    }

    for hook in hooks {
        unsafe {
            let _ = UnhookWinEvent(hook);
        }
    }
    unsafe {
        CoUninitialize();
    }
}

fn install_hooks() -> Vec<HWINEVENTHOOK> {
    WIN_EVENT_IDS
        .iter()
        .filter_map(|event_id| {
            let hook = unsafe {
                SetWinEventHook(
                    *event_id,
                    *event_id,
                    None,
                    Some(win_event_proc),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                )
            };
            (!hook.is_invalid()).then_some(hook)
        })
        .collect()
}

unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    idobject: i32,
    idchild: i32,
    _event_thread: u32,
    event_time_ms: u32,
) {
    let Ok(guard) = WIN_EVENT_SENDER.lock() else {
        return;
    };
    let Some(sender) = guard.as_ref() else {
        return;
    };
    let Some(kind) = event_kind(event) else {
        return;
    };
    invalidate_snapshot_cache();

    let window_id = hwnd.0 as isize as i64;
    let event = AccessibleEvent {
        seq: u64::from(event_time_ms),
        at_ms: u64::from(event_time_ms),
        window_id,
        element_id: event_element_id(hwnd, event, idobject, idchild),
        kind,
        name: None,
        value: None,
    };
    let _ = sender.send(event);
}

const fn event_kind(event: u32) -> Option<AccessibleEventKind> {
    match event {
        EVENT_SYSTEM_FOREGROUND => Some(AccessibleEventKind::ForegroundChanged),
        EVENT_OBJECT_FOCUS => Some(AccessibleEventKind::FocusChanged),
        EVENT_OBJECT_VALUECHANGE => Some(AccessibleEventKind::ValueChanged),
        EVENT_OBJECT_NAMECHANGE => Some(AccessibleEventKind::NameChanged),
        EVENT_OBJECT_CREATE => Some(AccessibleEventKind::ElementAppeared),
        EVENT_OBJECT_DESTROY => Some(AccessibleEventKind::ElementDisappeared),
        EVENT_OBJECT_SELECTION => Some(AccessibleEventKind::SelectionChanged),
        EVENT_SYSTEM_MENUSTART => Some(AccessibleEventKind::MenuStart),
        EVENT_SYSTEM_MENUEND => Some(AccessibleEventKind::MenuEnd),
        EVENT_SYSTEM_ALERT => Some(AccessibleEventKind::Alert),
        _ => None,
    }
}

fn event_element_id(hwnd: HWND, event: u32, idobject: i32, idchild: i32) -> Option<ElementId> {
    if hwnd.0.is_null() {
        return None;
    }
    let window_id = hwnd.0 as isize as i64;
    let runtime_id = format!(
        "{event:08x}{:08x}{:08x}",
        idobject.cast_unsigned(),
        idchild.cast_unsigned()
    );
    Some(element_id(window_id, &runtime_id))
}

fn read_current_apartment() -> ComApartmentKind {
    let mut apartment = APTTYPE::default();
    let mut qualifier = APTTYPEQUALIFIER::default();
    if unsafe { CoGetApartmentType(&raw mut apartment, &raw mut qualifier) }.is_err() {
        return ComApartmentKind::Unknown;
    }
    match apartment {
        value if value == APTTYPE_STA => ComApartmentKind::Sta,
        value if value == APTTYPE_MTA => ComApartmentKind::Mta,
        value if value == APTTYPE_NA => ComApartmentKind::Neutral,
        value if value == APTTYPE_MAINSTA => ComApartmentKind::MainSta,
        _ => ComApartmentKind::Unknown,
    }
}
