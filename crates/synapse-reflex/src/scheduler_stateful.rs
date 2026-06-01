use std::{collections::HashSet, time::Duration};

use synapse_core::{
    Action, ButtonAction, Event, Point, ReflexAimAxis, ReflexButtonTarget, error_codes,
};

use super::RuntimeState;
use crate::{
    ReflexError,
    conflict::{ConflictCandidate, ConflictLoser, resolve_conflicts},
    dispatch::ReflexActionDispatchContext,
    kinds::{
        aim_track::{AimTrackContext, AimTrackOutput, AimTrackTarget},
        combo::{ComboContext, ComboOutput, ComboPhase},
        hold_button::{HoldButtonOutput, HoldButtonPhase},
        hold_lifetime::HoldLifetimeContext,
        hold_move::{HoldMoveOutput, HoldMovePhase},
    },
    scheduler::ScheduledReflexDriver,
};

pub(super) fn step_stateful_controllers(
    runtime: &mut RuntimeState,
    events: &[Event],
    elapsed: Duration,
    dispatched_actions: &mut usize,
    dispatch_blocked: &mut bool,
    starvation_losers: &mut Vec<ConflictLoser>,
) {
    let controls = super::lock_controls(&runtime.controls).clone();
    let selection = resolve_stateful_conflicts(runtime, &controls);
    starvation_losers.extend(selection.losers);
    for index in 0..runtime.reflexes.len() {
        if !controls.get(index).is_some_and(|control| control.active) {
            continue;
        }
        if selection.blocked_slots.contains(&index) {
            continue;
        }

        for outcome in [
            step_aim_track(runtime, index, elapsed),
            step_hold_move(runtime, index, events, elapsed),
            step_hold_button(runtime, index, events, elapsed),
            step_combo(runtime, index, elapsed),
        ]
        .into_iter()
        .flatten()
        {
            match outcome {
                StatefulOutcome::Progressed { actions } => {
                    *dispatched_actions = dispatched_actions.saturating_add(actions);
                }
                StatefulOutcome::Fired { actions } => {
                    *dispatched_actions = dispatched_actions.saturating_add(actions);
                    super::mark_reflex_fired(runtime, index);
                }
                StatefulOutcome::Expired { actions, reason } => {
                    *dispatched_actions = dispatched_actions.saturating_add(actions);
                    super::mark_reflex_lifetime_expired(runtime, index, reason);
                }
                StatefulOutcome::Idle => {}
                StatefulOutcome::Blocked { error } => {
                    *dispatch_blocked = true;
                    if error.code() == error_codes::REFLEX_ACTION_PERMISSION_DENIED {
                        super::mark_reflex_action_denied(runtime, index);
                    } else {
                        super::mark_reflex_error(runtime, index, error.code());
                    }
                    warn_stateful_dispatch_blocked(index, &error);
                    return;
                }
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
struct StatefulConflictSelection {
    blocked_slots: HashSet<usize>,
    losers: Vec<ConflictLoser>,
}

fn resolve_stateful_conflicts(
    runtime: &RuntimeState,
    controls: &[super::ReflexControl],
) -> StatefulConflictSelection {
    let mut plans = Vec::new();
    for index in 0..runtime.reflexes.len() {
        if !controls.get(index).is_some_and(|control| control.active) {
            continue;
        }
        let actions = stateful_conflict_actions(runtime, index);
        if actions.is_empty() {
            continue;
        }
        plans.push(StatefulConflictPlan {
            reflex_index: index,
            actions,
        });
    }

    let candidates = plans
        .iter()
        .enumerate()
        .map(|(candidate_index, plan)| {
            let runtime_reflex = &runtime.reflexes[plan.reflex_index];
            let control = &controls[plan.reflex_index];
            ConflictCandidate::new(
                candidate_index,
                plan.reflex_index,
                runtime_reflex.reflex.reflex_id.clone(),
                control.priority,
                runtime_reflex.registration_order,
                runtime_reflex.reflex.exclusive,
                &plan.actions,
            )
        })
        .collect::<Vec<_>>();
    let resolution = resolve_conflicts(&candidates);
    let blocked_slots = resolution
        .losers
        .iter()
        .map(|loser| loser.loser_slot)
        .collect::<HashSet<_>>();

    StatefulConflictSelection {
        blocked_slots,
        losers: resolution.losers,
    }
}

#[derive(Clone, Debug)]
struct StatefulConflictPlan {
    reflex_index: usize,
    actions: Vec<Action>,
}

fn stateful_conflict_actions(runtime: &RuntimeState, index: usize) -> Vec<Action> {
    match &runtime.reflexes[index].reflex.driver {
        ScheduledReflexDriver::Actions => Vec::new(),
        ScheduledReflexDriver::AimTrack(_) => aim_track_conflict_actions(runtime, index),
        ScheduledReflexDriver::HoldMove(_) => hold_move_conflict_actions(runtime, index),
        ScheduledReflexDriver::HoldButton(_) => hold_button_conflict_actions(runtime, index),
        ScheduledReflexDriver::Combo(_) => combo_conflict_actions(runtime, index),
    }
}

fn aim_track_conflict_actions(runtime: &RuntimeState, index: usize) -> Vec<Action> {
    let Some(controller) = runtime.aim_track_states.get(index).and_then(Option::as_ref) else {
        return Vec::new();
    };
    let params = controller.params();
    let Some(target) = aim_static_target(&params.target) else {
        return Vec::new();
    };
    let Ok(cursor) = synapse_action::backend::software::cursor_position() else {
        return Vec::new();
    };
    if !aim_outside_deadzone(cursor, target, params.axis, params.deadzone_px) {
        return Vec::new();
    }
    vec![Action::MouseMoveRelative {
        dx: 0.0,
        dy: 0.0,
        backend: params.backend,
    }]
}

fn hold_move_conflict_actions(runtime: &RuntimeState, index: usize) -> Vec<Action> {
    let Some(controller) = runtime.hold_move_states.get(index).and_then(Option::as_ref) else {
        return Vec::new();
    };
    if !matches!(
        controller.phase(),
        HoldMovePhase::Pending | HoldMovePhase::Holding
    ) {
        return Vec::new();
    }
    controller
        .params()
        .keys
        .iter()
        .cloned()
        .map(|key| Action::KeyDown {
            key,
            backend: controller.params().backend,
        })
        .collect()
}

fn hold_button_conflict_actions(runtime: &RuntimeState, index: usize) -> Vec<Action> {
    let Some(controller) = runtime
        .hold_button_states
        .get(index)
        .and_then(Option::as_ref)
    else {
        return Vec::new();
    };
    if !matches!(
        controller.phase(),
        HoldButtonPhase::Pending | HoldButtonPhase::Holding
    ) {
        return Vec::new();
    }
    vec![hold_button_action(
        &controller.params().button,
        controller.params().backend,
    )]
}

fn combo_conflict_actions(runtime: &RuntimeState, index: usize) -> Vec<Action> {
    let Some(controller) = runtime.combo_states.get(index).and_then(Option::as_ref) else {
        return Vec::new();
    };
    if !matches!(
        controller.phase(),
        ComboPhase::Pending | ComboPhase::Running
    ) {
        return Vec::new();
    }
    let ScheduledReflexDriver::Combo(params) = &runtime.reflexes[index].reflex.driver else {
        return Vec::new();
    };
    vec![Action::Combo {
        steps: params.steps.clone(),
        backend: params.backend,
    }]
}

fn hold_button_action(button: &ReflexButtonTarget, backend: synapse_core::Backend) -> Action {
    match button {
        ReflexButtonTarget::Mouse { button } => Action::MouseButton {
            button: *button,
            action: ButtonAction::Down,
            hold_ms: 0,
            backend,
        },
        ReflexButtonTarget::Pad { pad, button } => Action::PadButton {
            pad: *pad,
            button: *button,
            action: ButtonAction::Down,
            hold_ms: 0,
        },
    }
}

fn aim_static_target(target: &AimTrackTarget) -> Option<Point> {
    match target {
        AimTrackTarget::Point(point) => Some(*point),
        AimTrackTarget::ElementRect(rect) => Some(Point {
            x: rect.x.saturating_add(rect.w / 2),
            y: rect.y.saturating_add(rect.h / 2),
        }),
        AimTrackTarget::EntityId(_) | AimTrackTarget::TrackId(_) | AimTrackTarget::ElementId(_) => {
            None
        }
    }
}

fn aim_outside_deadzone(
    cursor: Point,
    target: Point,
    axis: ReflexAimAxis,
    deadzone_px: f32,
) -> bool {
    let mut dx = f64::from(target.x) - f64::from(cursor.x);
    let mut dy = f64::from(target.y) - f64::from(cursor.y);
    match axis {
        ReflexAimAxis::Xy => {}
        ReflexAimAxis::XOnly => dy = 0.0,
        ReflexAimAxis::YOnly => dx = 0.0,
    }
    dx.hypot(dy) > f64::from(deadzone_px)
}

#[derive(Clone, Debug)]
enum StatefulOutcome {
    Progressed {
        actions: usize,
    },
    Fired {
        actions: usize,
    },
    Expired {
        actions: usize,
        reason: &'static str,
    },
    Idle,
    Blocked {
        error: ReflexError,
    },
}

fn step_combo(
    runtime: &mut RuntimeState,
    index: usize,
    elapsed: Duration,
) -> Option<StatefulOutcome> {
    let dispatch_context = dispatch_context(runtime);
    let reflex_id = runtime.reflexes[index].reflex.reflex_id.clone();
    let controller = runtime.combo_states.get_mut(index)?.as_mut()?;
    let context = ComboContext {
        tick_elapsed: elapsed,
    };
    match controller.step_dispatch_with(&context, &runtime.event_bus, |action| {
        dispatch_context.dispatch_action(&reflex_id, action)
    }) {
        Ok(ComboOutput::Completed { actions, .. }) => Some(StatefulOutcome::Expired {
            actions,
            reason: "completed",
        }),
        Ok(output) if output.action_count() > 0 => Some(StatefulOutcome::Progressed {
            actions: output.action_count(),
        }),
        Ok(
            ComboOutput::Idle { .. } | ComboOutput::Started { .. } | ComboOutput::Dispatched { .. },
        ) => Some(StatefulOutcome::Idle),
        Err(error) => Some(StatefulOutcome::Blocked { error }),
    }
}

fn dispatch_context(runtime: &RuntimeState) -> ReflexActionDispatchContext {
    ReflexActionDispatchContext::new(
        runtime.action_handle.clone(),
        runtime.action_gate.clone(),
        runtime.audit_db.clone(),
        runtime.audit_context.clone(),
        runtime.tick_index,
    )
}

fn step_aim_track(
    runtime: &mut RuntimeState,
    index: usize,
    elapsed: Duration,
) -> Option<StatefulOutcome> {
    let dispatch_context = dispatch_context(runtime);
    let reflex_id = runtime.reflexes[index].reflex.reflex_id.clone();
    let controller = runtime.aim_track_states.get_mut(index)?.as_mut()?;
    let cursor = match synapse_action::backend::software::cursor_position() {
        Ok(cursor) => cursor,
        Err(error) => {
            return Some(StatefulOutcome::Blocked {
                error: ReflexError::ParamsInvalid {
                    detail: format!("aim_track cursor read failed: {error}"),
                },
            });
        }
    };
    let context = AimTrackContext {
        cursor,
        entities: &[],
        elements: &[],
        tick_index: runtime.tick_index,
        tick_elapsed: elapsed,
    };
    match controller.step_dispatch_with(&context, &runtime.event_bus, |action| {
        dispatch_context.dispatch_action(&reflex_id, action)
    }) {
        Ok(AimTrackOutput::Dispatched { .. }) => Some(StatefulOutcome::Fired { actions: 1 }),
        Ok(AimTrackOutput::Idle { .. }) => Some(StatefulOutcome::Idle),
        Err(ReflexError::TrackLost { .. }) => Some(StatefulOutcome::Blocked {
            error: ReflexError::TrackLost {
                reflex_id: reflex_id.clone(),
            },
        }),
        Err(error) => Some(StatefulOutcome::Blocked { error }),
    }
}

fn step_hold_move(
    runtime: &mut RuntimeState,
    index: usize,
    events: &[Event],
    elapsed: Duration,
) -> Option<StatefulOutcome> {
    let dispatch_context = dispatch_context(runtime);
    let reflex_id = runtime.reflexes[index].reflex.reflex_id.clone();
    let controller = runtime.hold_move_states.get_mut(index)?.as_mut()?;
    let mut actions = 0_usize;
    let mut registered = false;
    if matches!(controller.phase(), HoldMovePhase::Pending) {
        match controller
            .register_dispatch_with(|action| dispatch_context.dispatch_action(&reflex_id, action))
        {
            Ok(HoldMoveOutput::Registered {
                actions: registered_actions,
            }) => {
                actions = actions.saturating_add(registered_actions);
                registered = true;
            }
            Ok(
                HoldMoveOutput::Holding { .. }
                | HoldMoveOutput::Released { .. }
                | HoldMoveOutput::Idle { .. },
            ) => {}
            Err(error) => return Some(StatefulOutcome::Blocked { error }),
        }
    }

    let context = HoldLifetimeContext {
        tick_elapsed: elapsed,
        events,
        cancelled: false,
    };
    match controller.step_dispatch_with(&context, &runtime.event_bus, |action| {
        dispatch_context.dispatch_action(&reflex_id, action)
    }) {
        Ok(
            HoldMoveOutput::Holding { .. }
            | HoldMoveOutput::Idle { .. }
            | HoldMoveOutput::Registered { .. },
        ) if registered => Some(StatefulOutcome::Fired { actions }),
        Ok(
            HoldMoveOutput::Holding { .. }
            | HoldMoveOutput::Idle { .. }
            | HoldMoveOutput::Registered { .. },
        ) => Some(StatefulOutcome::Idle),
        Ok(HoldMoveOutput::Released {
            actions: released_actions,
            ..
        }) => Some(StatefulOutcome::Expired {
            actions: actions.saturating_add(released_actions),
            reason: "released",
        }),
        Err(ReflexError::LifetimeExpired { .. }) => Some(StatefulOutcome::Expired {
            actions: actions.saturating_add(controller.params().keys.len()),
            reason: "lifetime",
        }),
        Err(error) => Some(StatefulOutcome::Blocked { error }),
    }
}

fn step_hold_button(
    runtime: &mut RuntimeState,
    index: usize,
    events: &[Event],
    elapsed: Duration,
) -> Option<StatefulOutcome> {
    let dispatch_context = dispatch_context(runtime);
    let reflex_id = runtime.reflexes[index].reflex.reflex_id.clone();
    let controller = runtime.hold_button_states.get_mut(index)?.as_mut()?;
    let mut actions = 0_usize;
    let mut registered = false;
    if matches!(controller.phase(), HoldButtonPhase::Pending) {
        match controller
            .register_dispatch_with(|action| dispatch_context.dispatch_action(&reflex_id, action))
        {
            Ok(HoldButtonOutput::Registered) => {
                actions = actions.saturating_add(1);
                registered = true;
            }
            Ok(
                HoldButtonOutput::Holding { .. }
                | HoldButtonOutput::Released { .. }
                | HoldButtonOutput::Idle { .. },
            ) => {}
            Err(error) => return Some(StatefulOutcome::Blocked { error }),
        }
    }

    let context = HoldLifetimeContext {
        tick_elapsed: elapsed,
        events,
        cancelled: false,
    };
    match controller.step_dispatch_with(&context, &runtime.event_bus, |action| {
        dispatch_context.dispatch_action(&reflex_id, action)
    }) {
        Ok(
            HoldButtonOutput::Holding { .. }
            | HoldButtonOutput::Idle { .. }
            | HoldButtonOutput::Registered,
        ) if registered => Some(StatefulOutcome::Fired { actions }),
        Ok(
            HoldButtonOutput::Holding { .. }
            | HoldButtonOutput::Idle { .. }
            | HoldButtonOutput::Registered,
        ) => Some(StatefulOutcome::Idle),
        Ok(HoldButtonOutput::Released { .. }) => Some(StatefulOutcome::Expired {
            actions: actions.saturating_add(1),
            reason: "released",
        }),
        Err(ReflexError::LifetimeExpired { .. }) => Some(StatefulOutcome::Expired {
            actions: actions.saturating_add(1),
            reason: "lifetime",
        }),
        Err(error) => Some(StatefulOutcome::Blocked { error }),
    }
}

fn warn_stateful_dispatch_blocked(index: usize, error: &ReflexError) {
    tracing::warn!(
        component = "reflex_scheduler",
        reflex_index = index,
        error_code = error.code(),
        detail = %error,
        code = error_codes::REFLEX_TICK_LATE,
        "reflex stateful controller dispatch blocked"
    );
}
