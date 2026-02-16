use std::sync::Arc;

use sim_backend::agent_core::{
    AgentCoreRepository, AgentTickExecutionStatus, AgentTickOrchestrator,
    AgentTickOrchestratorOutcome,
};
use sim_backend::app::config::WorkerConfig;
use sim_backend::app::observability::init_tracing;
use sim_backend::app::runtime::ServiceRuntime;
use sim_backend::infrastructure::gemini::GeminiClient;
use sim_backend::infrastructure::gemini_embedding::GeminiEmbeddingClient;
use sim_backend::infrastructure::postgres::{
    ensure_ready, PostgresAgentCoreRepository, PostgresMemoryRepository,
};
use sim_backend::infrastructure::qdrant::QdrantVectorStore;
use sim_backend::llm::LlmPort;
use sim_backend::memory::{
    MemoryRepository, MemoryService, MemoryVectorStore, SimpleHashEmbedder, TextEmbedder,
};

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

    let vector_store: Arc<dyn MemoryVectorStore> =
        Arc::new(QdrantVectorStore::new(config.qdrant.clone())?);
    vector_store.ensure_collection().await?;
    let memory_repository: Arc<dyn MemoryRepository> =
        Arc::new(PostgresMemoryRepository::new(db_pool.clone()));
    let embedder: Arc<dyn TextEmbedder> = match config.gemini.clone() {
        Some(gemini_config) => Arc::new(GeminiEmbeddingClient::new(gemini_config)?),
        None => Arc::new(SimpleHashEmbedder::new(config.qdrant.vector_size as usize)),
    };
    let memory_service = Arc::new(MemoryService::new(
        memory_repository,
        vector_store,
        embedder,
        gemini_client.clone(),
        config.qdrant.vector_size as usize,
    ));

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
    let tick_memory_service = Arc::clone(&memory_service);
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

                                        if let Err(error) = tick_memory_service
                                            .append_memory(
                                                result.agent_id,
                                                format!(
                                                    "{} (mood={}, valence={:.2}, arousal={:.2})",
                                                    result.action_summary,
                                                    result.mood_label,
                                                    result.valence,
                                                    result.arousal
                                                ),
                                                0.7,
                                            )
                                            .await
                                        {
                                            tracing::error!(
                                                agent_id = %result.agent_id,
                                                error = %error,
                                                "failed to append episodic memory from tick"
                                            );
                                        }
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

    let embed_interval = config.memory.embed_interval;
    let embed_batch = config.memory.embed_batch_size;
    let embed_service = Arc::clone(&memory_service);
    let embed_token = cancellation.clone();
    runtime.spawn("memory_embedding_worker", async move {
        let mut interval = tokio::time::interval(embed_interval);
        loop {
            tokio::select! {
                _ = embed_token.cancelled() => {
                    tracing::info!("memory embedding worker received shutdown");
                    break;
                }
                _ = interval.tick() => {
                    match embed_service.process_pending_embeddings(embed_batch).await {
                        Ok(summary) => {
                            if summary.processed > 0 {
                                tracing::info!(
                                    processed = summary.processed,
                                    succeeded = summary.succeeded,
                                    failed = summary.failed,
                                    "memory embeddings processed"
                                );
                            }
                        }
                        Err(error) => {
                            tracing::error!(error = %error, "memory embedding worker failed");
                        }
                    }
                }
            }
        }
        Ok(())
    });

    let summary_interval = config.memory.summary_interval;
    let summary_max_active = config.memory.max_active_per_agent;
    let summary_batch_size = config.memory.summary_batch_size;
    let summary_agent_ids = config.agent_ids.clone();
    let summary_service = Arc::clone(&memory_service);
    let summary_token = cancellation.clone();
    runtime.spawn("memory_summarization_worker", async move {
        let mut interval = tokio::time::interval(summary_interval);
        loop {
            tokio::select! {
                _ = summary_token.cancelled() => {
                    tracing::info!("memory summarization worker received shutdown");
                    break;
                }
                _ = interval.tick() => {
                    for agent_id in &summary_agent_ids {
                        match summary_service
                            .summarize_overflow(*agent_id, summary_max_active, summary_batch_size)
                            .await
                        {
                            Ok(result) if result.created_summary => {
                                tracing::info!(
                                    agent_id = %agent_id,
                                    source_count = result.source_count,
                                    summary_id = ?result.summary_entry_id,
                                    "memory overflow summarized"
                                );
                            }
                            Ok(_) => {}
                            Err(error) => {
                                tracing::error!(agent_id = %agent_id, error = %error, "memory summarization failed");
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    });

    runtime.run_until_shutdown().await
}
