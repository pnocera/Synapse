use std::time::Duration;

use synapse_core::{Action, Key, MouseButton, PadButton, PadId, ReflexId, Stick, Trigger};

pub const REFLEX_STARVED_KIND: &str = "reflex_starved";
pub const STARVATION_AFTER: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ConflictResource {
    KeyboardText,
    Key(Key),
    MouseCursor,
    MouseButton(MouseButton),
    PadButton { pad: PadId, button: PadButton },
    PadStick { pad: PadId, stick: Stick },
    PadTrigger { pad: PadId, trigger: Trigger },
    PadReport { pad: PadId },
}

impl ConflictResource {
    fn conflicts_with(&self, other: &Self) -> bool {
        if keyboard_conflict(self, other)
            || matches!((self, other), (Self::MouseCursor, Self::MouseCursor))
        {
            return true;
        }
        if let (Some(left), Some(right)) = (report_pad(self), pad_resource(other)) {
            return left == right;
        }
        if let (Some(left), Some(right)) = (pad_resource(self), report_pad(other)) {
            return left == right;
        }
        match (self, other) {
            (Self::Key(left), Self::Key(right)) => left == right,
            (Self::MouseButton(left), Self::MouseButton(right)) => left == right,
            (
                Self::PadButton {
                    pad: left_pad,
                    button: left_button,
                },
                Self::PadButton {
                    pad: right_pad,
                    button: right_button,
                },
            ) => left_pad == right_pad && left_button == right_button,
            (
                Self::PadStick {
                    pad: left_pad,
                    stick: left_stick,
                },
                Self::PadStick {
                    pad: right_pad,
                    stick: right_stick,
                },
            ) => left_pad == right_pad && left_stick == right_stick,
            (
                Self::PadTrigger {
                    pad: left_pad,
                    trigger: left_trigger,
                },
                Self::PadTrigger {
                    pad: right_pad,
                    trigger: right_trigger,
                },
            ) => left_pad == right_pad && left_trigger == right_trigger,
            _ => false,
        }
    }

    fn label(&self) -> String {
        match self {
            Self::KeyboardText => "keyboard_text".to_owned(),
            Self::Key(key) => format!("key:{:?}", key.code),
            Self::MouseCursor => "mouse_cursor".to_owned(),
            Self::MouseButton(button) => format!("mouse_button:{button:?}"),
            Self::PadButton { pad, button } => format!("pad:{pad}:button:{button:?}"),
            Self::PadStick { pad, stick } => format!("pad:{pad}:stick:{stick:?}"),
            Self::PadTrigger { pad, trigger } => format!("pad:{pad}:trigger:{trigger:?}"),
            Self::PadReport { pad } => format!("pad:{pad}:report"),
        }
    }
}

const fn keyboard_conflict(left: &ConflictResource, right: &ConflictResource) -> bool {
    matches!(
        (left, right),
        (
            ConflictResource::KeyboardText,
            ConflictResource::KeyboardText | ConflictResource::Key(_)
        ) | (ConflictResource::Key(_), ConflictResource::KeyboardText)
    )
}

const fn report_pad(resource: &ConflictResource) -> Option<PadId> {
    match resource {
        ConflictResource::PadReport { pad } => Some(*pad),
        _ => None,
    }
}

const fn pad_resource(resource: &ConflictResource) -> Option<PadId> {
    match resource {
        ConflictResource::PadButton { pad, .. }
        | ConflictResource::PadStick { pad, .. }
        | ConflictResource::PadTrigger { pad, .. }
        | ConflictResource::PadReport { pad } => Some(*pad),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ConflictCandidate {
    pub candidate_index: usize,
    pub reflex_slot: usize,
    pub reflex_id: ReflexId,
    pub priority: u32,
    pub registration_order: usize,
    pub resources: Vec<ConflictResource>,
}

impl ConflictCandidate {
    #[must_use]
    pub(crate) fn new(
        candidate_index: usize,
        reflex_slot: usize,
        reflex_id: ReflexId,
        priority: u32,
        registration_order: usize,
        actions: &[Action],
    ) -> Self {
        Self {
            candidate_index,
            reflex_slot,
            reflex_id,
            priority,
            registration_order,
            resources: action_resources(actions),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConflictLoser {
    pub candidate_index: usize,
    pub loser_slot: usize,
    pub loser_reflex_id: ReflexId,
    pub winner_slot: usize,
    pub winner_reflex_id: ReflexId,
    pub resource: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ConflictResolution {
    pub winners: Vec<usize>,
    pub losers: Vec<ConflictLoser>,
}

#[must_use]
pub(crate) fn resolve_conflicts(candidates: &[ConflictCandidate]) -> ConflictResolution {
    let mut resolution = ConflictResolution::default();
    for candidate in candidates {
        if candidate.resources.is_empty() {
            resolution.winners.push(candidate.candidate_index);
            continue;
        }

        let blocker = candidates
            .iter()
            .filter(|other| other.candidate_index != candidate.candidate_index)
            .filter(|other| outranks(other, candidate))
            .filter_map(|other| {
                contested_resource(other, candidate).map(|resource| (other, resource))
            })
            .min_by(|(left, _), (right, _)| compare_precedence(left, right));

        if let Some((winner, resource)) = blocker {
            resolution.losers.push(ConflictLoser {
                candidate_index: candidate.candidate_index,
                loser_slot: candidate.reflex_slot,
                loser_reflex_id: candidate.reflex_id.clone(),
                winner_slot: winner.reflex_slot,
                winner_reflex_id: winner.reflex_id.clone(),
                resource: resource.label(),
            });
        } else {
            resolution.winners.push(candidate.candidate_index);
        }
    }

    resolution.winners.sort_by(|left, right| {
        let left_candidate = &candidates[*left];
        let right_candidate = &candidates[*right];
        compare_precedence(left_candidate, right_candidate)
    });
    resolution
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct StarvationState {
    contended_for: Duration,
    reported: bool,
}

impl StarvationState {
    #[must_use]
    pub(crate) fn record_loss(&mut self, elapsed: Duration) -> bool {
        self.contended_for = self.contended_for.saturating_add(elapsed);
        if self.contended_for >= STARVATION_AFTER && !self.reported {
            self.reported = true;
            return true;
        }
        false
    }

    pub(crate) const fn reset(&mut self) {
        self.contended_for = Duration::ZERO;
        self.reported = false;
    }

    #[must_use]
    pub(crate) const fn contended_for(&self) -> Duration {
        self.contended_for
    }
}

fn action_resources(actions: &[Action]) -> Vec<ConflictResource> {
    actions.iter().flat_map(action_resource).collect::<Vec<_>>()
}

fn action_resource(action: &Action) -> Vec<ConflictResource> {
    match action {
        Action::KeyPress { key, .. } | Action::KeyDown { key, .. } | Action::KeyUp { key, .. } => {
            vec![ConflictResource::Key(key.clone())]
        }
        Action::KeyChord { keys, .. } => keys
            .iter()
            .cloned()
            .map(ConflictResource::Key)
            .collect::<Vec<_>>(),
        Action::TypeText { .. } => vec![ConflictResource::KeyboardText],
        Action::MouseMove { .. }
        | Action::MouseMoveRelative { .. }
        | Action::MouseScroll { .. }
        | Action::AimAt { .. } => vec![ConflictResource::MouseCursor],
        Action::MouseButton { button, .. } => vec![ConflictResource::MouseButton(*button)],
        Action::MouseDrag { button, .. } => vec![
            ConflictResource::MouseCursor,
            ConflictResource::MouseButton(*button),
        ],
        Action::PadButton { pad, button, .. } => {
            vec![ConflictResource::PadButton {
                pad: *pad,
                button: *button,
            }]
        }
        Action::PadStick { pad, stick, .. } => vec![ConflictResource::PadStick {
            pad: *pad,
            stick: *stick,
        }],
        Action::PadTrigger { pad, trigger, .. } => vec![ConflictResource::PadTrigger {
            pad: *pad,
            trigger: *trigger,
        }],
        Action::PadReport { pad, .. } => vec![ConflictResource::PadReport { pad: *pad }],
        Action::Combo { .. } | Action::ReleaseAll => Vec::new(),
    }
}

fn contested_resource<'a>(
    stronger: &'a ConflictCandidate,
    weaker: &'a ConflictCandidate,
) -> Option<&'a ConflictResource> {
    stronger.resources.iter().find(|stronger_resource| {
        weaker
            .resources
            .iter()
            .any(|weaker_resource| stronger_resource.conflicts_with(weaker_resource))
    })
}

const fn outranks(left: &ConflictCandidate, right: &ConflictCandidate) -> bool {
    left.priority < right.priority
        || (left.priority == right.priority && left.registration_order > right.registration_order)
}

fn compare_precedence(left: &ConflictCandidate, right: &ConflictCandidate) -> std::cmp::Ordering {
    left.priority
        .cmp(&right.priority)
        .then_with(|| right.registration_order.cmp(&left.registration_order))
}
