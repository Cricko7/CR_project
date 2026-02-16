use anyhow::Context;
use tracing_subscriber::EnvFilter;

pub fn init_tracing(service_name: &str, default_level: &str) -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(default_level))
        .with_context(|| format!("failed to build tracing filter from level `{default_level}`"))?;

    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .with_target(true)
        .with_current_span(true)
        .with_span_list(true)
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .context("failed to install tracing subscriber")?;

    tracing::info!(service = service_name, "tracing initialized");
    Ok(())
}
