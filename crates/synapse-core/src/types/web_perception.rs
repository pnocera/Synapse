//! Web/browser perception diagnostics shared across crates.
//!
//! When the foreground window is a Chromium-family browser, Synapse attaches to
//! CDP to read the page's DOM/accessibility tree.
//! These types describe *whether* that attach succeeded and *which* perception
//! path produced the observed web content, so an agent can reason about fidelity
//! instead of silently receiving a collapsed UIA-only tree (see epic #682, the
//! #683 regression, and the #687 strategy ladder).
//!
//! They live in `synapse-core` (not `synapse-a11y`) because they are embedded in
//! [`crate::ObservationDiagnostics`], which every crate that touches an
//! `Observation` must see. `synapse-a11y` owns the actual probe/attach logic and
//! re-exports these types.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Which perception path produced the web/document content for a Chromium-family
/// foreground window.
///
/// Reported in `observe` diagnostics (`web_path`) so callers can reason about
/// fidelity. `None` (absent) means the foreground is not a browser and the field
/// does not apply.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WebPerceptionPath {
    /// CDP attached: the DOM/accessibility tree is the source of truth. Best
    /// fidelity — every link/button/textbox/heading is queryable.
    Cdp,
    /// CDP unreachable: content recovered via pixel capture + OCR tiling of the
    /// content region. Degraded but non-empty.
    Ocr,
    /// CDP unreachable and OCR not applied: only the collapsed UIA
    /// window/pane/region tree is available. This is the trap #682 documents —
    /// it is always accompanied by a non-`ok` [`CdpStatus`] so the caller knows
    /// why web content is missing and how to fix it.
    UiaOnly,
}

impl WebPerceptionPath {
    /// Stable wire string for logs/diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cdp => "cdp",
            Self::Ocr => "ocr",
            Self::UiaOnly => "uia_only",
        }
    }
}

/// Outcome of probing/attaching CDP for a Chromium-family foreground window.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CdpStatus {
    /// A debug endpoint was reachable (and, where attempted, attached).
    Ok,
    /// The foreground process is not a Chromium-family browser; CDP does not
    /// apply.
    NotChromium,
    /// The foreground is Chromium-family but no remote-debugging port was
    /// reachable.
    #[serde(rename = "A11Y_CDP_UNREACHABLE")]
    Unreachable,
    /// A debug port was reachable but the CDP client failed to attach or read
    /// the tree.
    #[serde(rename = "A11Y_CDP_ATTACH_FAILED")]
    AttachFailed,
}

impl CdpStatus {
    /// Stable wire string for logs/diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::NotChromium => "not_chromium",
            Self::Unreachable => "A11Y_CDP_UNREACHABLE",
            Self::AttachFailed => "A11Y_CDP_ATTACH_FAILED",
        }
    }
}

/// CDP capabilities Synapse exercises once attached. Surfaced so an agent knows
/// which queries are available on the attached tab.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CdpCapability {
    DomSnapshot,
    AccessibilityFullAxTree,
    DomQuerySelector,
    PageCaptureScreenshot,
}

/// Diagnostics describing the CDP probe/attach outcome for the current
/// foreground. Embedded in [`crate::ObservationDiagnostics::cdp`] for every
/// Chromium-family foreground (and `None` otherwise).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CdpDiagnostics {
    /// Foreground process image name that was probed (e.g. `chrome.exe`).
    pub process_name: String,
    /// Probe/attach outcome.
    pub status: CdpStatus,
    /// Reachable/attached endpoint (`http://127.0.0.1:<port>`), when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Machine-readable reason code for non-`ok` statuses
    /// (`A11Y_CDP_UNREACHABLE`, `A11Y_CDP_ATTACH_FAILED`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    /// Human-readable detail for attach failures (the underlying error).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Capabilities available on the attached tab (empty unless `status == ok`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<CdpCapability>,
    /// Number of DOM/AX nodes surfaced into `elements` from the attached tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attached_node_count: Option<u32>,
}

impl CdpDiagnostics {
    /// Foreground is not a Chromium-family browser.
    #[must_use]
    pub fn not_chromium(process_name: impl Into<String>) -> Self {
        Self {
            process_name: process_name.into(),
            status: CdpStatus::NotChromium,
            endpoint: None,
            reason_code: None,
            detail: None,
            capabilities: Vec::new(),
            attached_node_count: None,
        }
    }

    /// Chromium-family but no reachable debug port.
    #[must_use]
    pub fn unreachable(process_name: impl Into<String>, reason_code: impl Into<String>) -> Self {
        Self {
            process_name: process_name.into(),
            status: CdpStatus::Unreachable,
            endpoint: None,
            reason_code: Some(reason_code.into()),
            detail: None,
            capabilities: Vec::new(),
            attached_node_count: None,
        }
    }
}
