mod auth;
mod session;
mod transport;

pub async fn serve(bind: &str) -> anyhow::Result<std::process::ExitCode> {
    transport::serve(bind).await
}
