mod a11y_events;
pub mod audio;
pub mod permissions;
pub mod profile;
pub mod reflex;
pub mod replay;
pub mod subscribe;
#[cfg(test)]
mod tests;
use anyhow::{Result, bail};
use std::{
    num::NonZeroUsize,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use synapse_action::ActionHandle;
use synapse_audio::{AudioConfig, AudioError, AudioRuntime, DEFAULT_RING_SECONDS};
use synapse_core::SCHEMA_VERSION;
use synapse_profiles::{ProfileError, ProfileRuntime, bundled_profiles_dir};
use synapse_reflex::{DEFAULT_MAX_SUBSCRIPTIONS_NONZERO, EventBus, ReflexError, ReflexRuntime};
use synapse_storage::Db;
use tokio_util::sync::CancellationToken;

use self::a11y_events::A11yEventBridge;
use self::permissions::{PermissionGrants, configured_grants_from_parts};
use crate::http::sse::SseState;

const DB_ENV: &str = "SYNAPSE_DB";
const PROFILE_DIR_ENV: &str = "SYNAPSE_PROFILE_DIR";
const REFLEX_DISABLED_ENV: &str = "SYNAPSE_REFLEX_DISABLED";
const ENABLE_AUDIO_ENV: &str = "SYNAPSE_ENABLE_AUDIO";
const ALLOW_UNKNOWN_PROFILE_ENV: &str = "SYNAPSE_ALLOW_UNKNOWN_PROFILE";
const ALLOWED_PERMISSIONS_ENV: &str = "SYNAPSE_MCP_ALLOWED_PERMISSIONS";
const BIND_ENV: &str = "SYNAPSE_BIND";
const BEARER_TOKEN_ENV: &str = "SYNAPSE_BEARER_TOKEN";
const AUDIO_LOOPBACK_ENV: &str = "SYNAPSE_AUDIO_LOOPBACK";
const MAX_SUBSCRIPTIONS_ENV: &str = "SYNAPSE_MAX_SUBSCRIPTIONS";
const DEFAULT_BIND: &str = "127.0.0.1:7700";
pub type SharedM3State = Arc<Mutex<M3State>>;

#[derive(Clone, Debug)]
pub struct M3ServiceConfig {
    pub db_path: Option<PathBuf>,
    pub profile_dir: Option<PathBuf>,
    pub reflex_disabled: bool,
    pub bind: String,
    pub bearer_token: Option<String>,
    pub max_subscriptions: NonZeroUsize,
    pub enable_audio: bool,
    pub allow_unknown_profile: bool,
    pub allowed_permissions: Option<String>,
}

impl M3ServiceConfig {
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "constructor mirrors parsed CLI/config fields without hiding startup gates"
    )]
    pub fn from_cli_parts(
        db_path: Option<PathBuf>,
        profile_dir: Option<PathBuf>,
        reflex_disabled: bool,
        bind: String,
        max_subscriptions: NonZeroUsize,
        enable_audio: bool,
        allow_unknown_profile: bool,
        allowed_permissions: Option<String>,
    ) -> Self {
        Self {
            db_path,
            profile_dir,
            reflex_disabled,
            bind,
            bearer_token: std::env::var(BEARER_TOKEN_ENV).ok(),
            max_subscriptions,
            enable_audio,
            allow_unknown_profile,
            allowed_permissions,
        }
    }

    pub fn from_env() -> Result<Self> {
        let reflex_disabled_raw = std::env::var(REFLEX_DISABLED_ENV).ok();
        let enable_audio_raw = std::env::var(ENABLE_AUDIO_ENV).ok();
        let allow_unknown_profile_raw = std::env::var(ALLOW_UNKNOWN_PROFILE_ENV).ok();
        let max_subscriptions_raw = std::env::var(MAX_SUBSCRIPTIONS_ENV).ok();
        Ok(Self {
            db_path: std::env::var_os(DB_ENV).map(PathBuf::from),
            profile_dir: std::env::var_os(PROFILE_DIR_ENV).map(PathBuf::from),
            reflex_disabled: parse_bool_env(REFLEX_DISABLED_ENV, reflex_disabled_raw.as_deref())?,
            enable_audio: parse_bool_env(ENABLE_AUDIO_ENV, enable_audio_raw.as_deref())?,
            allow_unknown_profile: parse_bool_env(
                ALLOW_UNKNOWN_PROFILE_ENV,
                allow_unknown_profile_raw.as_deref(),
            )?,
            bind: std::env::var(BIND_ENV).unwrap_or_else(|_| DEFAULT_BIND.to_owned()),
            bearer_token: std::env::var(BEARER_TOKEN_ENV).ok(),
            max_subscriptions: parse_max_subscriptions_env(max_subscriptions_raw.as_deref())?,
            allowed_permissions: std::env::var(ALLOWED_PERMISSIONS_ENV).ok(),
        })
    }
}

#[derive(Debug)]
pub struct M3State {
    pub db_path: Option<PathBuf>,
    pub profile_dir: Option<PathBuf>,
    pub reflex_disabled: bool,
    pub bind: String,
    pub bearer_token: Option<String>,
    pub shutdown_cancel: CancellationToken,
    pub shutdown_reason: &'static str,
    pub connection_closed_cancel: Option<CancellationToken>,
    pub permission_grants: PermissionGrants,
    pub enable_audio: bool,
    pub allow_unknown_profile: bool,
    pub profile_runtime: Option<Arc<ProfileRuntime>>,
    pub sse_state: SseState,
    pub reflex_runtime: Option<Arc<Mutex<ReflexRuntime>>>,
    pub a11y_event_bridge: Option<A11yEventBridge>,
    pub audio_runtime: Option<Arc<AudioRuntime>>,
}

pub fn shared_m3_state_from_env() -> Result<SharedM3State> {
    Ok(Arc::new(Mutex::new(M3State::from_config(
        M3ServiceConfig::from_env()?,
    )?)))
}

pub fn shared_m3_state_from_config_with_shutdown_reason_and_sse_state(
    config: M3ServiceConfig,
    shutdown_cancel: CancellationToken,
    shutdown_reason: &'static str,
    connection_closed_cancel: Option<CancellationToken>,
    sse_state: SseState,
) -> Result<SharedM3State> {
    Ok(Arc::new(Mutex::new(
        M3State::from_config_with_shutdown_reason_and_sse_state(
            config,
            shutdown_cancel,
            shutdown_reason,
            connection_closed_cancel,
            sse_state,
        )?,
    )))
}

impl M3State {
    pub fn from_config(config: M3ServiceConfig) -> Result<Self> {
        let sse_state = SseState::with_max_subscriptions(config.max_subscriptions);
        Self::from_config_with_shutdown_reason_and_sse_state(
            config,
            CancellationToken::new(),
            "shutdown",
            None,
            sse_state,
        )
    }

    pub fn from_config_with_shutdown_reason_and_sse_state(
        config: M3ServiceConfig,
        shutdown_cancel: CancellationToken,
        shutdown_reason: &'static str,
        connection_closed_cancel: Option<CancellationToken>,
        sse_state: SseState,
    ) -> Result<Self> {
        Self::from_parts_with_sse_state(
            config.db_path,
            config.profile_dir,
            Some(bool_env_value(config.reflex_disabled)),
            config.bearer_token,
            Some(config.bind),
            Some(bool_env_value(config.enable_audio)),
            Some(bool_env_value(config.allow_unknown_profile)),
            config.allowed_permissions.as_deref(),
            shutdown_cancel,
            shutdown_reason,
            connection_closed_cancel,
            sse_state,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_parts_with_sse_state(
        db_path: Option<PathBuf>,
        profile_dir: Option<PathBuf>,
        reflex_disabled: Option<&str>,
        bearer_token: Option<String>,
        bind: Option<String>,
        enable_audio: Option<&str>,
        allow_unknown_profile: Option<&str>,
        allowed_permissions: Option<&str>,
        shutdown_cancel: CancellationToken,
        shutdown_reason: &'static str,
        connection_closed_cancel: Option<CancellationToken>,
        sse_state: SseState,
    ) -> Result<Self> {
        let enable_audio = parse_bool_env(ENABLE_AUDIO_ENV, enable_audio)?;
        let allow_unknown_profile =
            parse_bool_env(ALLOW_UNKNOWN_PROFILE_ENV, allow_unknown_profile)?;
        let permission_grants = configured_grants_from_parts(allowed_permissions, enable_audio)?;
        Ok(Self {
            db_path,
            profile_dir,
            reflex_disabled: parse_bool_env(REFLEX_DISABLED_ENV, reflex_disabled)?,
            bind: bind
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| DEFAULT_BIND.to_owned()),
            bearer_token: bearer_token.filter(|value| !value.is_empty()),
            shutdown_cancel,
            shutdown_reason,
            connection_closed_cancel,
            permission_grants,
            enable_audio,
            allow_unknown_profile,
            profile_runtime: None,
            sse_state,
            reflex_runtime: None,
            a11y_event_bridge: None,
            audio_runtime: None,
        })
    }

    #[must_use]
    pub const fn scaffold_ready(&self) -> bool {
        !self.bind.is_empty()
    }

    pub fn ensure_profile_runtime(
        &mut self,
    ) -> std::result::Result<Arc<ProfileRuntime>, ProfileError> {
        if let Some(runtime) = &self.profile_runtime {
            return Ok(Arc::clone(runtime));
        }

        let profile_dir = self
            .profile_dir
            .clone()
            .unwrap_or_else(bundled_profiles_dir);
        let runtime = Arc::new(ProfileRuntime::spawn(profile_dir)?);
        self.profile_runtime = Some(Arc::clone(&runtime));
        Ok(runtime)
    }

    pub fn ensure_reflex_runtime(
        &mut self,
        action_handle: ActionHandle,
        event_bus: EventBus,
    ) -> Result<Arc<Mutex<ReflexRuntime>>> {
        if let Some(runtime) = &self.reflex_runtime {
            return Ok(Arc::clone(runtime));
        }
        if self.reflex_disabled {
            bail!(ReflexError::DisabledByOperator {
                detail: "SYNAPSE_REFLEX_DISABLED is set".to_owned(),
            });
        }

        let db_path = self.db_path.clone().unwrap_or_else(default_db_path);
        let db = Arc::new(Db::open(&db_path, SCHEMA_VERSION)?);
        let runtime = Arc::new(Mutex::new(ReflexRuntime::spawn(
            db,
            action_handle,
            event_bus,
        )?));
        self.reflex_runtime = Some(Arc::clone(&runtime));
        Ok(runtime)
    }

    pub fn ensure_a11y_event_bridge(&mut self, event_bus: EventBus) -> Result<()> {
        if self.a11y_event_bridge.is_some() {
            return Ok(());
        }
        let bridge =
            A11yEventBridge::start(event_bus).map_err(|error| ReflexError::ParamsInvalid {
                detail: format!("a11y event bridge start failed: {error}"),
            })?;
        self.a11y_event_bridge = Some(bridge);
        Ok(())
    }

    pub fn ensure_audio_runtime(&mut self) -> std::result::Result<Arc<AudioRuntime>, AudioError> {
        if let Some(runtime) = &self.audio_runtime {
            return Ok(Arc::clone(runtime));
        }

        let config = AudioConfig {
            ring_seconds: DEFAULT_RING_SECONDS,
            start_loopback: audio_loopback_enabled()?,
            detectors_enabled: false,
            stt_model_path: None,
        };
        let runtime = Arc::new(AudioRuntime::spawn(config)?);
        self.audio_runtime = Some(Arc::clone(&runtime));
        Ok(runtime)
    }
}

fn default_db_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("synapse")
        .join("db")
}
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct M3ToolStub {
    pub name: &'static str,
}

impl M3ToolStub {
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }
}

#[must_use]
pub const fn m3_tool_stubs() -> [M3ToolStub; 11] {
    [
        subscribe::subscribe(),
        subscribe::subscribe_cancel(),
        reflex::reflex_register(),
        reflex::reflex_cancel(),
        reflex::reflex_list(),
        reflex::reflex_history(),
        profile::profile_list(),
        profile::profile_activate(),
        replay::replay_record(),
        audio::audio_tail(),
        audio::audio_transcribe(),
    ]
}

fn parse_bool_env(name: &str, value: Option<&str>) -> Result<bool> {
    match value {
        None | Some("0") => Ok(false),
        Some("1") => Ok(true),
        Some(value) if value.eq_ignore_ascii_case("true") => Ok(true),
        Some(value) if value.eq_ignore_ascii_case("false") => Ok(false),
        Some(value) => bail!("{name} must be one of 1, 0, true, or false; got {value:?}"),
    }
}

fn parse_max_subscriptions_env(value: Option<&str>) -> Result<NonZeroUsize> {
    let Some(value) = value else {
        return Ok(DEFAULT_MAX_SUBSCRIPTIONS_NONZERO);
    };
    parse_max_subscriptions_value(MAX_SUBSCRIPTIONS_ENV, value)
}

fn parse_max_subscriptions_value(name: &str, value: &str) -> Result<NonZeroUsize> {
    let trimmed = value.trim();
    let Ok(parsed) = trimmed.parse::<usize>() else {
        bail!("{name} must be a positive integer; got {value:?}");
    };
    let Some(nonzero) = NonZeroUsize::new(parsed) else {
        bail!("{name} must be >= 1; got {value:?}");
    };
    Ok(nonzero)
}

const fn bool_env_value(value: bool) -> &'static str {
    if value { "1" } else { "0" }
}

fn audio_loopback_enabled() -> std::result::Result<bool, AudioError> {
    match std::env::var(AUDIO_LOOPBACK_ENV).ok().as_deref() {
        None | Some("1") => Ok(true),
        Some("0") => Ok(false),
        Some(value) if value.eq_ignore_ascii_case("true") => Ok(true),
        Some(value) if value.eq_ignore_ascii_case("false") => Ok(false),
        Some(value) => Err(AudioError::LoopbackInitFailed {
            detail: format!(
                "{AUDIO_LOOPBACK_ENV} must be one of 1, 0, true, or false; got {value:?}"
            ),
        }),
    }
}
