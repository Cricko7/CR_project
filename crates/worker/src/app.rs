use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Utc;
use serde_json::json;
use sim_backend::agent_core::{
    AgentCoreRepository, AgentRecord, AgentTickExecutionStatus, AgentTickOrchestrator,
    AgentTickOrchestratorOutcome, DEFAULT_SIMULATION_TIME_SCALE, MAX_SIMULATION_TIME_SCALE,
    MIN_SIMULATION_TIME_SCALE, MessageRecord, NewAgentEvent, NewMessage,
};
use sim_backend::app::config::WorkerConfig;
use sim_backend::app::observability::init_tracing;
use sim_backend::app::runtime::ServiceRuntime;
use sim_backend::infrastructure::gemini::GeminiClient;
use sim_backend::infrastructure::gemini_embedding::GeminiEmbeddingClient;
use sim_backend::infrastructure::ollama::OllamaClient;
use sim_backend::infrastructure::openrouter::OpenRouterClient;
use sim_backend::infrastructure::postgres::{
    PostgresAgentCoreRepository, PostgresMemoryRepository, ensure_ready,
};
use sim_backend::infrastructure::qdrant::QdrantVectorStore;
use sim_backend::llm::{FallbackLlm, LlmPort};
use sim_backend::memory::{
    MemoryRepository, MemoryService, MemoryVectorStore, SimpleHashEmbedder, TextEmbedder,
};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use uuid::Uuid;

mod constants;
mod conversations;
mod processing;
mod random;
mod simulation;
mod workers;

use constants::*;
use conversations::*;
use processing::*;
use random::*;
use simulation::*;
use workers::*;
pub async fn run() -> anyhow::Result<()> {
    let config = WorkerConfig::from_env()?;
    init_tracing(&config.common.service_name, &config.common.log_level)?;
    let db_pool = ensure_ready(&config.database).await?;
    let gemini_client: Option<Arc<dyn LlmPort>> = match config.gemini.clone() {
        Some(gemini_config) => {
            tracing::info!(model = %gemini_config.model, "gemini client configured");
            Some(Arc::new(GeminiClient::new(gemini_config)?) as Arc<dyn LlmPort>)
        }
        None => None,
    };
    let openrouter_client: Option<Arc<dyn LlmPort>> = match config.openrouter.clone() {
        Some(openrouter_config) => {
            tracing::info!(model = %openrouter_config.model, "openrouter client configured");
            Some(Arc::new(OpenRouterClient::new(openrouter_config)?) as Arc<dyn LlmPort>)
        }
        None => None,
    };
    let ollama_client: Option<Arc<dyn LlmPort>> = match config.ollama.clone() {
        Some(ollama_config) => {
            tracing::info!(model = %ollama_config.model, "ollama client configured");
            Some(Arc::new(OllamaClient::new(ollama_config)?) as Arc<dyn LlmPort>)
        }
        None => None,
    };
    let llm_client: Option<Arc<dyn LlmPort>> = match (
        gemini_client,
        openrouter_client,
        ollama_client,
    ) {
        (Some(gemini), Some(openrouter), Some(ollama)) => {
            tracing::info!(
                "gemini primary llm with openrouter fallback and ollama final fallback configured"
            );
            let openrouter_chain =
                Arc::new(FallbackLlm::new(openrouter, ollama, "openrouter", "ollama"))
                    as Arc<dyn LlmPort>;
            Some(Arc::new(FallbackLlm::new(
                gemini,
                openrouter_chain,
                "gemini",
                "openrouter->ollama",
            )) as Arc<dyn LlmPort>)
        }
        (Some(gemini), Some(openrouter), None) => {
            tracing::info!("gemini primary llm with openrouter fallback configured");
            Some(
                Arc::new(FallbackLlm::new(gemini, openrouter, "gemini", "openrouter"))
                    as Arc<dyn LlmPort>,
            )
        }
        (Some(gemini), None, Some(ollama)) => {
            tracing::info!("gemini primary llm with ollama fallback configured");
            Some(Arc::new(FallbackLlm::new(gemini, ollama, "gemini", "ollama")) as Arc<dyn LlmPort>)
        }
        (None, Some(openrouter), Some(ollama)) => {
            tracing::warn!(
                "GEMINI_API_KEY is not set; openrouter primary with ollama fallback is configured"
            );
            Some(
                Arc::new(FallbackLlm::new(openrouter, ollama, "openrouter", "ollama"))
                    as Arc<dyn LlmPort>,
            )
        }
        (Some(gemini), None, None) => {
            tracing::info!(
                "openrouter and ollama are not configured; gemini is used as the only llm"
            );
            Some(gemini)
        }
        (None, Some(openrouter), None) => {
            tracing::warn!("GEMINI_API_KEY is not set; openrouter is used as primary llm");
            Some(openrouter)
        }
        (None, None, Some(ollama)) => {
            tracing::warn!(
                "GEMINI_API_KEY and OPENROUTER_API_KEY are not set; ollama is used as primary llm"
            );
            Some(ollama)
        }
        (None, None, None) => {
            tracing::warn!(
                "GEMINI_API_KEY, OPENROUTER_API_KEY and OLLAMA_MODEL are not set; worker runs without llm integration"
            );
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
        llm_client.clone(),
        config.qdrant.vector_size as usize,
    ));

    let postgres_agent_repository = Arc::new(PostgresAgentCoreRepository::new(db_pool));
    let repository: Arc<dyn AgentCoreRepository> = postgres_agent_repository.clone();
    let orchestrator = AgentTickOrchestrator::new(repository.clone()).with_optional_llm(llm_client);

    let mut runtime = ServiceRuntime::new(
        config.common.service_name.clone(),
        config.common.shutdown_timeout,
    );
    spawn_agent_tick_worker(
        &mut runtime,
        config.agent_ids.clone(),
        config.tick_interval,
        config.tick_concurrency as usize,
        orchestrator.clone(),
        Arc::clone(&memory_service),
        repository.clone(),
    );

    spawn_mood_decay_worker(
        &mut runtime,
        config.mood_decay_interval,
        config.mood_decay_step,
        postgres_agent_repository.clone(),
    );

    spawn_memory_embedding_worker(
        &mut runtime,
        config.memory.embed_interval,
        config.memory.embed_batch_size,
        Arc::clone(&memory_service),
    );

    spawn_memory_summarization_worker(
        &mut runtime,
        config.memory.summary_interval,
        config.memory.max_active_per_agent,
        config.memory.summary_batch_size,
        config.agent_ids.clone(),
        Arc::clone(&memory_service),
    );

    spawn_message_delivery_worker(
        &mut runtime,
        config.message_interval,
        config.message_batch_size,
        repository.clone(),
    );

    spawn_conversation_seed_worker(
        &mut runtime,
        repository.clone(),
        config.conversation_scan_interval,
        config.conversation_min_interval,
        config.conversation_max_interval,
        config.conversation_agent_limit,
    );

    runtime.run_until_shutdown().await
}
