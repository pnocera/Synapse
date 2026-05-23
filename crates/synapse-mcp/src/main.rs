mod m1;
mod m2;
mod server;

use std::{path::PathBuf, process::ExitCode, time::Duration};

use anyhow::Context;
use clap::{Parser, ValueEnum};
use rmcp::ServiceExt;
use synapse_telemetry::{TelemetryConfig, TelemetryGuard, init_tracing};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::filter::LevelFilter;

use crate::server::SynapseService;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Mode {
    Stdio,
    Http,
}

#[derive(Debug, Parser)]
#[command(name = "synapse-mcp", version, about = "Synapse MCP daemon")]
struct Cli {
    #[arg(long, value_enum, default_value_t = Mode::Stdio, env = "SYNAPSE_MODE")]
    mode: Mode,
    #[arg(long, default_value = "127.0.0.1:7700", env = "SYNAPSE_BIND")]
    bind: String,
    #[arg(long, env = "SYNAPSE_DB")]
    db: Option<PathBuf>,
    #[arg(long, env = "SYNAPSE_PROFILE_DIR")]
    profile_dir: Option<PathBuf>,
    #[arg(long, env = "SYNAPSE_LOG_LEVEL", default_value = "info")]
    log_level: String,
    #[arg(long, env = "SYNAPSE_REFLEX_DISABLED")]
    reflex_disabled: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => code,
        Err(err) => {
            eprintln!("synapse-mcp error: {err:#}");
            ExitCode::from(1)
        }
    }
}

async fn run() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();
    let telemetry_guard = configure_telemetry(&cli)?;
    let dpi_awareness = synapse_capture::init_process_dpi_awareness()
        .context("initialize per-monitor DPI awareness")?;
    tracing::info!(?cli, code = "MCP_CLI_PARSED", "synapse-mcp cli parsed");
    tracing::info!(
        ?dpi_awareness,
        code = "CAPTURE_DPI_AWARENESS_INITIALIZED",
        "capture dpi awareness initialized"
    );

    match cli.mode {
        Mode::Stdio => run_stdio(telemetry_guard).await,
        Mode::Http => {
            tracing::error!(code = "NOT_YET_IMPLEMENTED", "--mode http lands at M3");
            drop(telemetry_guard);
            eprintln!("NOT_YET_IMPLEMENTED: --mode http lands at M3");
            Ok(ExitCode::from(2))
        }
    }
}

fn configure_telemetry(cli: &Cli) -> anyhow::Result<TelemetryGuard> {
    let level = cli
        .log_level
        .parse::<LevelFilter>()
        .with_context(|| format!("invalid log level {}", cli.log_level))?;
    let log_dir = std::env::var_os("SYNAPSE_LOG_DIR").map(PathBuf::from);
    init_tracing(TelemetryConfig {
        log_dir,
        file_level: level,
        console_level: level,
        ..TelemetryConfig::default()
    })
    .context("initialize telemetry")
}

async fn run_stdio(telemetry_guard: TelemetryGuard) -> anyhow::Result<ExitCode> {
    tracing::info!(code = "MCP_STDIO_STARTED", "starting stdio MCP transport");
    let rmcp_token = CancellationToken::new();
    let emitter_shutdown_token = CancellationToken::new();
    let emitter_connection_closed_token = CancellationToken::new();
    let service = SynapseService::with_m2_shutdown_reason(
        emitter_shutdown_token.clone(),
        "sigint",
        emitter_connection_closed_token.clone(),
    );
    synapse_action::install_panic_hook();
    let m2_emitter_done = service.m2_emitter_done_receiver();
    let start = service.serve_with_ct(rmcp::transport::stdio(), rmcp_token.clone());
    tokio::pin!(start);
    let service = tokio::select! {
        service = &mut start => match service {
            Ok(service) => service,
            Err(err) if err.to_string().contains("connection closed") => {
                tracing::info!(code = "MCP_STDIO_CLOSED_BEFORE_INIT", "stdio closed before init");
                drop(telemetry_guard);
                return Ok(ExitCode::SUCCESS);
            }
            Err(err) => return Err(err).context("start rmcp stdio service"),
        },
        signal = tokio::signal::ctrl_c() => {
            signal.context("wait for ctrl-c during startup")?;
            rmcp_token.cancel();
            emitter_shutdown_token.cancel();
            tracing::info!(code = "MCP_SHUTDOWN_GRACEFUL", "shutdown signal received before init");
            drop(telemetry_guard);
            std::process::exit(0);
        }
    };
    let shutdown = service.cancellation_token();
    let mut wait_task = tokio::spawn(async move { service.waiting().await });

    let code = tokio::select! {
        wait = &mut wait_task => {
            wait.context("join rmcp service")??;
            emitter_connection_closed_token.cancel();
            wait_for_m2_emitter_done(m2_emitter_done).await;
            ExitCode::SUCCESS
        }
        signal = tokio::signal::ctrl_c() => {
            signal.context("wait for ctrl-c")?;
            tracing::info!(code = "MCP_SHUTDOWN_GRACEFUL", "shutdown signal received");
            emitter_shutdown_token.cancel();
            shutdown.cancel();
            if let Ok(wait) = tokio::time::timeout(Duration::from_secs(5), &mut wait_task).await {
                wait.context("join rmcp service after shutdown")??;
                wait_for_m2_emitter_done(m2_emitter_done).await;
                drop(telemetry_guard);
                std::process::exit(0);
            } else {
                tracing::error!(code = "MCP_SHUTDOWN_TIMEOUT", "shutdown timeout");
                drop(telemetry_guard);
                std::process::exit(124);
            }
        }
    };

    drop(telemetry_guard);
    Ok(code)
}

async fn wait_for_m2_emitter_done(
    done: Option<tokio::sync::watch::Receiver<Option<synapse_action::ActionStateSnapshot>>>,
) {
    let Some(mut done) = done else {
        return;
    };
    let _wait_result = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if done.borrow().is_some() {
                break;
            }
            if done.changed().await.is_err() {
                break;
            }
        }
    })
    .await;
}
