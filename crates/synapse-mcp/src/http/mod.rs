mod auth;
mod transport;

pub async fn serve(bind: &str) -> anyhow::Result<std::process::ExitCode> {
    transport::serve(bind).await
}
