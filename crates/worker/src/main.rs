use std::sync::Arc;

use sim_backend::agent_core::{
    AgentCoreRepository, AgentTickExecutionStatus, AgentTickOrchestrator,
    AgentTickOrchestratorOutcome,
};
use sim_backend::app::config::WorkerConfig;
use sim_backend::app::observability::init_tracing;
use sim_backend::app::runtime::ServiceRuntime;
use sim_backend::infrastructure::gemini::GeminiClient;
use sim_backend::infrastructure::postgres::{ensure_ready, PostgresAgentCoreRepository};
use sim_backend::llm::LlmPort;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = WorkerConfig::from_env()?;
    init_tracing(&config.common.service_name, &config.common.log_level)?;
    let db_pool = ensure_ready(&config.database).await?;
    let gemini_client: Option<Arc<dyn LlmPort>> = match config.gemini.clone() {
        Some(gemini_config) => {
            tracing::info!(model = %gemini_config.model, "gemini client configured");
            Some(Arc::new(GeminiClient::new(gemini_config)?) as Arc<dyn LlmPort>)
        }
        None => {
            tracing::warn!("GEMINI_API_KEY is not set; worker runs without llm integration");
            None
        }
    };
    let repository: Arc<dyn AgentCoreRepository> =
        Arc::new(PostgresAgentCoreRepository::new(db_pool));
    let orchestrator = AgentTickOrchestrator::new(repository).with_optional_llm(gemini_client);

    let mut runtime = ServiceRuntime::new(
        config.common.service_name.clone(),
        config.common.shutdown_timeout,
    );
    let cancellation = runtime.cancellation_token();

    let tick_interval = config.tick_interval;
    let agent_ids = config.agent_ids.clone();
    let tick_orchestrator = orchestrator.clone();
    let tick_token = cancellation.clone();
    runtime.spawn("agent_tick_worker", async move {
        if agent_ids.is_empty() {
            tracing::warn!(
                "WORKER_AGENT_IDS is empty; tick worker is idle until agent ids are configured"
            );
        }

        let mut interval = tokio::time::interval(tick_interval);
        loop {
            tokio::select! {
                _ = tick_token.cancelled() => {
                    tracing::info!("agent tick worker received shutdown");
                    break;
                }
                _ = interval.tick() => {
                    for agent_id in &agent_ids {
                        match tick_orchestrator.run_agent_tick(*agent_id, None).await {
                            Ok(AgentTickOrchestratorOutcome::Executed(result)) => {
                                match result.status {
                                    AgentTickExecutionStatus::Applied => {
                                        tracing::info!(
                                            agent_id = %result.agent_id,
                                            tick_id = %result.tick_id,
                                            event_id = ?result.event_id,
                                            mood = %result.mood_label,
                                            "agent tick applied"
                                        );
                                    }
                                    AgentTickExecutionStatus::AgentMissing => {
                                        tracing::warn!(
                                            agent_id = %result.agent_id,
                                            tick_id = %result.tick_id,
                                            "agent not found for tick"
                                        );
                                    }
                                }
                            }
                            Ok(AgentTickOrchestratorOutcome::SkippedBusy) => {
                                tracing::debug!(agent_id = %agent_id, "skipped busy agent tick");
                            }
                            Ok(AgentTickOrchestratorOutcome::SkippedDuplicate) => {
                                tracing::debug!(agent_id = %agent_id, "skipped duplicate agent tick");
                            }
                            Err(error) => {
                                tracing::error!(agent_id = %agent_id, error = %error, "agent tick failed");
                            }
                        }
                    }
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
