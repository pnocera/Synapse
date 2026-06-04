use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

use chrono::Utc;
use synapse_core::{
    AccessibleNode, AudioContext, CaptureRuntimeReadback, CdpDiagnostics, ClipboardSummary,
    DetectedEntity, EventSummary, FocusedElement, ForegroundContext, FsEvent, HudReadings,
    Observation, ObservationCaptureConfig, ObservationDiagnostics, ObservationElementsPage,
    PerceptionMode, SensorStatus, WebPerceptionPath,
};

use crate::{PerceptionError, PerceptionResult};

const DEFAULT_MAX_ELEMENTS: usize = 60;
const DEFAULT_MAX_DEPTH: u32 = 2;
const DEFAULT_MAX_ENTITIES: usize = 60;
const SPARSE_A11Y_NODE_THRESHOLD: usize = 2;
const SPARSE_A11Y_DEPTH_THRESHOLD: u32 = 1;
const SENSOR_KEYS: [&str; 5] = ["a11y", "capture", "detection", "ocr", "audio"];

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct ObserveInclude {
    pub focused: bool,
    pub elements: bool,
    pub entities: bool,
    pub hud: bool,
    pub audio: bool,
    pub events: bool,
    pub clipboard: bool,
    pub fs: bool,
    pub diagnostics: bool,
    pub max_subtree_depth: u32,
    pub max_subtree_nodes: usize,
    pub element_offset: usize,
    pub max_entities: usize,
}

impl Default for ObserveInclude {
    fn default() -> Self {
        Self {
            focused: true,
            elements: true,
            entities: true,
            hud: true,
            audio: false,
            events: true,
            clipboard: false,
            fs: false,
            diagnostics: true,
            max_subtree_depth: DEFAULT_MAX_DEPTH,
            max_subtree_nodes: DEFAULT_MAX_ELEMENTS,
            element_offset: 0,
            max_entities: DEFAULT_MAX_ENTITIES,
        }
    }
}

impl ObserveInclude {
    #[must_use]
    pub const fn focused_only() -> Self {
        Self {
            focused: true,
            elements: false,
            entities: false,
            hud: false,
            audio: false,
            events: false,
            clipboard: false,
            fs: false,
            diagnostics: true,
            max_subtree_depth: DEFAULT_MAX_DEPTH,
            max_subtree_nodes: DEFAULT_MAX_ELEMENTS,
            element_offset: 0,
            max_entities: DEFAULT_MAX_ENTITIES,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct A11yTreeSummary {
    pub node_count: usize,
    pub max_depth: u32,
}

impl A11yTreeSummary {
    #[must_use]
    pub fn from_nodes(nodes: &[AccessibleNode]) -> Self {
        let max_depth = nodes
            .iter()
            .map(|node| node.depth)
            .max()
            .unwrap_or_default();
        Self {
            node_count: nodes.len(),
            max_depth,
        }
    }

    #[must_use]
    pub const fn is_sparse(&self) -> bool {
        self.node_count < SPARSE_A11Y_NODE_THRESHOLD || self.max_depth < SPARSE_A11Y_DEPTH_THRESHOLD
    }
}

#[derive(Clone, Debug)]
pub struct ObservationInput {
    pub foreground: ForegroundContext,
    pub focused: Option<FocusedElement>,
    pub elements: Vec<AccessibleNode>,
    pub entities: Vec<DetectedEntity>,
    pub hud: HudReadings,
    pub audio: AudioContext,
    pub recent_events: Vec<EventSummary>,
    pub clipboard_summary: Option<ClipboardSummary>,
    pub fs_recent: Vec<FsEvent>,
    pub sensor_latency_ms: BTreeMap<String, f32>,
    pub a11y_status: SensorStatus,
    pub capture_status: SensorStatus,
    pub detection_status: SensorStatus,
    pub audio_status: SensorStatus,
    pub mode_override: Option<PerceptionMode>,
    pub capture_config: Option<ObservationCaptureConfig>,
    pub capture_runtime: Option<CaptureRuntimeReadback>,
    /// CDP probe/attach outcome for the foreground (Chromium-family only).
    /// Threaded into [`ObservationDiagnostics::cdp`] by [`assemble`].
    pub cdp: Option<CdpDiagnostics>,
    /// Which perception path produced web content (Chromium-family only).
    /// Threaded into [`ObservationDiagnostics::web_path`] by [`assemble`].
    pub web_path: Option<WebPerceptionPath>,
}

impl ObservationInput {
    #[must_use]
    pub fn new(foreground: ForegroundContext) -> Self {
        Self {
            foreground,
            focused: None,
            elements: Vec::new(),
            entities: Vec::new(),
            hud: HudReadings::default(),
            audio: AudioContext::default(),
            recent_events: Vec::new(),
            clipboard_summary: None,
            fs_recent: Vec::new(),
            sensor_latency_ms: BTreeMap::new(),
            a11y_status: SensorStatus::Unavailable,
            capture_status: SensorStatus::Unavailable,
            detection_status: SensorStatus::Disabled,
            audio_status: SensorStatus::Disabled,
            mode_override: None,
            capture_config: None,
            capture_runtime: None,
            cdp: None,
            web_path: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct ObservationAssembler {
    next_seq: AtomicU64,
}

impl ObservationAssembler {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            next_seq: AtomicU64::new(1),
        }
    }

    /// Fuses current perception producer state into one `Observation`.
    ///
    /// # Errors
    ///
    /// Returns `OBSERVE_NO_PERCEPTION_AVAILABLE` when all sensor inputs are
    /// unavailable or disabled, and `OBSERVE_INTERNAL` when serialization fails.
    pub fn assemble(
        &self,
        include: ObserveInclude,
        input: ObservationInput,
    ) -> PerceptionResult<Observation> {
        let started = Instant::now();
        ensure_any_sensor_available(&input)?;
        let summary = A11yTreeSummary::from_nodes(&input.elements);
        let mode = input
            .mode_override
            .unwrap_or_else(|| auto_mode_with_a11y(&input.foreground, &summary));
        let cdp = input.cdp.clone();
        let web_path = input.web_path;
        let (elements, elements_truncated, elements_page) =
            filter_elements(input.elements, include);
        let (entities, entities_truncated) = filter_entities(input.entities, include);
        let mut observation = Observation {
            seq: self.next_seq.fetch_add(1, Ordering::Relaxed),
            at: Utc::now(),
            mode,
            foreground: input.foreground,
            focused: include.focused.then_some(input.focused).flatten(),
            elements,
            entities,
            hud: if include.hud {
                input.hud
            } else {
                HudReadings::default()
            },
            audio: if include.audio {
                input.audio
            } else {
                AudioContext::default()
            },
            recent_events: if include.events {
                input.recent_events
            } else {
                Vec::new()
            },
            clipboard_summary: include
                .clipboard
                .then_some(input.clipboard_summary)
                .flatten(),
            fs_recent: if include.fs {
                input.fs_recent
            } else {
                Vec::new()
            },
            diagnostics: ObservationDiagnostics {
                assembled_in_ms: started.elapsed().as_secs_f32() * 1000.0,
                sensor_latency_ms: bounded_sensor_latency(input.sensor_latency_ms),
                a11y_enabled: include.focused || include.elements || include.events,
                pixel_enabled: include.entities || include.hud,
                audio_enabled: include.audio,
                a11y_status: input.a11y_status,
                capture_status: input.capture_status,
                detection_status: input.detection_status,
                audio_status: input.audio_status,
                capture_config: input.capture_config,
                capture_runtime: input.capture_runtime,
                cdp,
                web_path,
                elements_truncated,
                elements_page,
                entities_truncated,
                size_bytes: 0,
                size_estimate_tokens: 0,
            },
        };
        update_size_fields(&mut observation)?;
        update_size_fields(&mut observation)?;
        Ok(observation)
    }
}

/// Assembles one observation with a fresh sequence counter.
///
/// # Errors
///
/// Returns the same errors as [`ObservationAssembler::assemble`].
pub fn assemble(include: ObserveInclude, input: ObservationInput) -> PerceptionResult<Observation> {
    ObservationAssembler::new().assemble(include, input)
}

/// Assembles one observation using default include filters.
///
/// # Errors
///
/// Returns the same errors as [`ObservationAssembler::assemble`].
pub fn assemble_from_input(input: ObservationInput) -> PerceptionResult<Observation> {
    assemble(ObserveInclude::default(), input)
}

#[must_use]
pub fn auto_mode(foreground: &ForegroundContext) -> PerceptionMode {
    if is_known_game_process(&foreground.process_name) {
        PerceptionMode::Hybrid
    } else {
        PerceptionMode::A11yOnly
    }
}

#[must_use]
pub fn auto_mode_with_a11y(
    foreground: &ForegroundContext,
    summary: &A11yTreeSummary,
) -> PerceptionMode {
    if is_known_game_process(&foreground.process_name) || summary.is_sparse() {
        PerceptionMode::Hybrid
    } else {
        PerceptionMode::A11yOnly
    }
}

/// Parses a manual perception-mode override.
///
/// # Errors
///
/// Returns `PERCEPTION_MODE_INVALID` for unknown strings.
pub fn parse_perception_mode(value: &str) -> PerceptionResult<PerceptionMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "a11y_only" => Ok(PerceptionMode::A11yOnly),
        "pixel_only" => Ok(PerceptionMode::PixelOnly),
        "hybrid" => Ok(PerceptionMode::Hybrid),
        "auto" => Ok(PerceptionMode::Auto),
        _ => Err(PerceptionError::PerceptionModeInvalid {
            value: value.to_owned(),
        }),
    }
}

#[must_use]
pub fn bounded_sensor_latency(input: BTreeMap<String, f32>) -> BTreeMap<String, f32> {
    input
        .into_iter()
        .filter(|(key, value)| SENSOR_KEYS.contains(&key.as_str()) && value.is_finite())
        .collect()
}

#[must_use]
pub fn is_known_game_process(process_name: &str) -> bool {
    matches!(
        process_name.to_ascii_lowercase().as_str(),
        "eldenring.exe"
            | "fortniteclient-win64-shipping.exe"
            | "game.exe"
            | "minecraft.exe"
            | "overwatch.exe"
            | "starfield.exe"
            | "valorant.exe"
    )
}

fn filter_elements(
    mut elements: Vec<AccessibleNode>,
    include: ObserveInclude,
) -> (Vec<AccessibleNode>, bool, Option<ObservationElementsPage>) {
    let original_total = elements.len();
    if !include.elements {
        let truncated = !elements.is_empty();
        let page = truncated.then(|| ObservationElementsPage {
            total: original_total,
            offset: 0,
            limit: 0,
            next_offset: Some(0),
        });
        return (Vec::new(), truncated, page);
    }
    let before = elements.len();
    elements.retain(|node| node.depth <= include.max_subtree_depth);
    let depth_truncated = elements.len() != before;
    let total = elements.len();
    let offset = include.element_offset.min(total);
    let limit = include.max_subtree_nodes;
    let next_offset = offset.checked_add(limit).filter(|next| *next < total);
    let paged = next_offset.is_some();
    let page = Some(ObservationElementsPage {
        total,
        offset,
        limit,
        next_offset,
    });
    let elements = elements.into_iter().skip(offset).take(limit).collect();
    (elements, depth_truncated || paged, page)
}

fn filter_entities(
    mut entities: Vec<DetectedEntity>,
    include: ObserveInclude,
) -> (Vec<DetectedEntity>, bool) {
    if !include.entities {
        return (Vec::new(), !entities.is_empty());
    }
    let truncated = entities.len() > include.max_entities;
    if truncated {
        entities.truncate(include.max_entities);
    }
    (entities, truncated)
}

fn ensure_any_sensor_available(input: &ObservationInput) -> PerceptionResult<()> {
    let statuses = [
        &input.a11y_status,
        &input.capture_status,
        &input.detection_status,
        &input.audio_status,
    ];
    if statuses.iter().any(|status| {
        matches!(
            status,
            SensorStatus::Healthy | SensorStatus::DegradedLatency { .. }
        )
    }) {
        return Ok(());
    }
    Err(PerceptionError::ObserveNoPerceptionAvailable {
        detail: "all perception producers unavailable or disabled".to_owned(),
    })
}

fn update_size_fields(observation: &mut Observation) -> PerceptionResult<()> {
    let size_bytes = u32::try_from(
        serde_json::to_vec(observation)
            .map_err(|err| PerceptionError::ObserveInternal {
                detail: err.to_string(),
            })?
            .len(),
    )
    .unwrap_or(u32::MAX);
    observation.diagnostics.size_bytes = size_bytes;
    observation.diagnostics.size_estimate_tokens = size_bytes.div_ceil(4);
    Ok(())
}
