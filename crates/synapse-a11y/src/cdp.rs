use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use synapse_core::error_codes;
use tokio::{net::TcpStream, time::timeout};

use crate::{A11yError, A11yResult};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CdpDiagnostics {
    pub process_name: String,
    pub status: CdpStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<CdpCapability>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CdpStatus {
    Ok,
    NotChromium,
    Unreachable,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CdpCapability {
    DomSnapshot,
    AccessibilityFullAxTree,
    DomQuerySelector,
    PageCaptureScreenshot,
}

#[must_use]
pub fn cdp_capabilities() -> Vec<CdpCapability> {
    vec![
        CdpCapability::DomSnapshot,
        CdpCapability::AccessibilityFullAxTree,
        CdpCapability::DomQuerySelector,
        CdpCapability::PageCaptureScreenshot,
    ]
}

#[must_use]
pub fn is_chromium_family(process_name: &str) -> bool {
    let lower = process_name.to_ascii_lowercase();
    [
        "chrome.exe",
        "chromium.exe",
        "msedge.exe",
        "brave.exe",
        "vivaldi.exe",
        "opera.exe",
        "chrome",
        "chromium",
        "msedge",
        "brave",
        "vivaldi",
        "opera",
    ]
    .iter()
    .any(|candidate| lower.ends_with(candidate))
}

pub async fn probe_chromium_cdp(
    process_name: &str,
    ports: &[u16],
    connect_timeout: Duration,
) -> CdpDiagnostics {
    if !is_chromium_family(process_name) {
        return CdpDiagnostics {
            process_name: process_name.to_owned(),
            status: CdpStatus::NotChromium,
            endpoint: None,
            reason_code: None,
            capabilities: Vec::new(),
        };
    }

    for port in ports {
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), *port);
        if timeout(connect_timeout, TcpStream::connect(addr))
            .await
            .is_ok_and(|result| result.is_ok())
        {
            return CdpDiagnostics {
                process_name: process_name.to_owned(),
                status: CdpStatus::Ok,
                endpoint: Some(format!("http://127.0.0.1:{port}")),
                reason_code: None,
                capabilities: cdp_capabilities(),
            };
        }
    }

    CdpDiagnostics {
        process_name: process_name.to_owned(),
        status: CdpStatus::Unreachable,
        endpoint: None,
        reason_code: Some(error_codes::A11Y_CDP_UNREACHABLE.to_owned()),
        capabilities: Vec::new(),
    }
}

#[cfg(windows)]
#[derive(Debug)]
pub struct CdpAttachment {
    pub browser: chromiumoxide::Browser,
    pub handler: chromiumoxide::Handler,
    pub endpoint: String,
}

/// Attaches a `chromiumoxide` browser client to a reachable CDP endpoint.
///
/// # Errors
///
/// Returns `A11Y_CDP_UNREACHABLE` when `chromiumoxide` cannot connect to the
/// supplied endpoint.
#[cfg(windows)]
pub async fn attach_chromiumoxide(endpoint: &str) -> A11yResult<CdpAttachment> {
    let (browser, handler) = chromiumoxide::Browser::connect(endpoint)
        .await
        .map_err(|err| A11yError::CdpUnreachable {
            detail: err.to_string(),
        })?;
    Ok(CdpAttachment {
        browser,
        handler,
        endpoint: endpoint.to_owned(),
    })
}
