use sim_backend::app::config::WorkerConfig;
use sim_backend::app::observability::init_tracing;
use sim_backend::app::runtime::ServiceRuntime;
use sim_backend::infrastructure::postgres::ensure_ready;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = WorkerConfig::from_env()?;
    init_tracing(&config.common.service_name, &config.common.log_level)?;
    let _db_pool = ensure_ready(&config.database).await?;

    let mut runtime = ServiceRuntime::new(
        config.common.service_name.clone(),
        config.common.shutdown_timeout,
    );
    let cancellation = runtime.cancellation_token();

    let tick_interval = config.tick_interval;
    let tick_token = cancellation.clone();
    runtime.spawn("agent_tick_worker", async move {
        let mut interval = tokio::time::interval(tick_interval);
        loop {
            tokio::select! {
                _ = tick_token.cancelled() => {
                    tracing::info!("agent tick worker received shutdown");
                    break;
                }
                _ = interval.tick() => {
                    tracing::info!("agent tick");
                }
            }
        }
        Ok(())
    });

    let mood_decay_interval = config.mood_decay_interval;
    let mood_token = cancellation.clone();
    runtime.spawn("mood_decay_worker", async move {
        let mut interval = tokio::time::interval(mood_decay_interval);
        loop {
            tokio::select! {
                _ = mood_token.cancelled() => {
                    tracing::info!("mood decay worker received shutdown");
                    break;
                }
                _ = interval.tick() => {
                    tracing::debug!("mood decay tick");
                }
            }
        }
        Ok(())
    });

    runtime.run_until_shutdown().await
}
