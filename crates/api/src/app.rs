use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::extract::{
    ConnectInfo, Path, Query, State,
    ws::{Message, WebSocket, WebSocketUpgrade},
};
use axum::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use axum::http::{Method, Request, StatusCode, Uri};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router, body::Body};
use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sim_backend::agent_core::{
    AgentCoreRepository, AgentEventRecord, AgentTickExecutionStatus, AgentTickOrchestrator,
    AgentTickOrchestratorOutcome, InterventionRecord, MAX_SIMULATION_TIME_SCALE,
    MIN_SIMULATION_TIME_SCALE, MessageRecord, NewAgentEvent, NewIntervention, NewMessage,
    RelationshipRecord, SimulationTimeScaleRecord,
};
use sim_backend::app::config::ApiConfig;
use sim_backend::app::observability::init_tracing;
use sim_backend::app::runtime::ServiceRuntime;
use sim_backend::infrastructure::gemini::GeminiClient;
use sim_backend::infrastructure::gemini_embedding::GeminiEmbeddingClient;
use sim_backend::infrastructure::openrouter::OpenRouterClient;
use sim_backend::infrastructure::postgres::{
    PostgresAgentCoreRepository, PostgresMemoryRepository, ensure_ready,
};
use sim_backend::infrastructure::qdrant::QdrantVectorStore;
use sim_backend::llm::{FallbackLlm, LlmPort};
use sim_backend::memory::{
    MemoryEntryRecord, MemoryRecallItem, MemoryRepository, MemoryService, MemoryVectorStore,
    SimpleHashEmbedder, TextEmbedder,
};
use sqlx::Row;
use tokio::sync::{Mutex, broadcast};
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

mod auth;
mod constants;
mod dto;
mod event_hub;
mod handlers;
mod limiter;
mod state;
mod utils;

use auth::*;
use constants::*;
use dto::*;
use event_hub::*;
use handlers::*;
use limiter::*;
use state::*;
use utils::*;
pub async fn run() -> anyhow::Result<()> {
    let config = ApiConfig::from_env().context("failed to load API config")?;
    init_tracing(&config.common.service_name, &config.common.log_level)?;
    let db_pool = ensure_ready(&config.database)
        .await
        .context("database startup check failed")?;
    let auth_repository = Arc::new(PostgresAuthRepository::new(db_pool.clone()));
    auth_repository
        .ensure_schema()
        .await
        .context("failed to initialize auth schema")?;

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
    let llm_client: Option<Arc<dyn LlmPort>> = match (gemini_client, openrouter_client) {
        (Some(gemini), Some(openrouter)) => {
            tracing::info!("gemini primary llm with openrouter fallback configured");
            Some(
                Arc::new(FallbackLlm::new(gemini, openrouter, "gemini", "openrouter"))
                    as Arc<dyn LlmPort>,
            )
        }
        (Some(gemini), None) => {
            tracing::info!("openrouter is not configured; gemini is used as the only llm");
            Some(gemini)
        }
        (None, Some(openrouter)) => {
            tracing::warn!("GEMINI_API_KEY is not set; openrouter is used as primary llm");
            Some(openrouter)
        }
        (None, None) => {
            tracing::warn!(
                "GEMINI_API_KEY and OPENROUTER_API_KEY are not set; api runs with deterministic tick fallback"
            );
            None
        }
    };

    let repository: Arc<dyn AgentCoreRepository> =
        Arc::new(PostgresAgentCoreRepository::new(db_pool.clone()));
    let orchestrator =
        AgentTickOrchestrator::new(repository.clone()).with_optional_llm(llm_client.clone());

    let vector_store: Arc<dyn MemoryVectorStore> =
        Arc::new(QdrantVectorStore::new(config.qdrant.clone())?);
    vector_store.ensure_collection().await?;
    let memory_repository: Arc<dyn MemoryRepository> =
        Arc::new(PostgresMemoryRepository::new(db_pool));
    let embedder: Arc<dyn TextEmbedder> = match config.gemini.clone() {
        Some(gemini_config) => Arc::new(GeminiEmbeddingClient::new(gemini_config)?),
        None => Arc::new(SimpleHashEmbedder::new(config.qdrant.vector_size as usize)),
    };
    let memory_service = Arc::new(MemoryService::new(
        memory_repository,
        vector_store,
        embedder,
        llm_client,
        config.qdrant.vector_size as usize,
    ));

    let mut runtime = ServiceRuntime::new(
        config.common.service_name.clone(),
        config.common.shutdown_timeout,
    );
    let cancellation = runtime.cancellation_token();
    let event_hub = ApiEventHub::new(EVENT_HUB_CAPACITY);

    let state = ApiState {
        service_name: config.common.service_name.clone(),
        repository: repository.clone(),
        orchestrator,
        memory_service,
        memory_defaults: MemoryRuntimeDefaults {
            summary_max_active: config.memory.max_active_per_agent,
            summary_batch_size: config.memory.summary_batch_size,
        },
        event_hub: event_hub.clone(),
    };
    let auth_state = AuthState {
        manager: Arc::new(AuthManager::new(
            config.auth_jwt_secret.clone(),
            config.auth_access_token_ttl,
            config.auth_refresh_token_ttl,
        )),
        repository: auth_repository.clone(),
    };
    let rate_limit_state = RateLimitState {
        limiter: Arc::new(IpRateLimiter::new(config.rate_limit_requests_per_minute)),
    };
    let app_state = AppState {
        api: state,
        auth: auth_state,
        rate_limit: rate_limit_state,
    };
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers([AUTHORIZATION, CONTENT_TYPE, ACCEPT]);
    let app = Router::new()
        .route("/health", get(health))
        .route("/livez", get(health))
        .route("/auth/register", post(auth_register))
        .route("/auth/login", post(auth_login))
        .route("/auth/refresh", post(auth_refresh))
        .route("/agents/{id}/ticks", post(trigger_agent_tick))
        .route("/agents/{id}/state", get(get_agent_state))
        .route("/agents/{id}/inspector", get(get_agent_inspector))
        .route("/events", get(list_events))
        .route(
            "/simulation/time-scale",
            get(get_simulation_time_scale).post(set_simulation_time_scale),
        )
        .route(
            "/interventions",
            post(create_intervention).get(list_interventions),
        )
        .route("/relationships/graph", get(get_relationship_graph))
        .route("/agents/{id}/memories", post(append_agent_memory))
        .route("/agents/{id}/memories/recall", get(recall_agent_memory))
        .route(
            "/agents/{id}/messages",
            post(send_agent_message).get(list_agent_messages),
        )
        .route("/agents/{id}/relationships", get(list_agent_relationships))
        .route(
            "/agents/{id}/relationships/history",
            get(list_agent_relationship_history),
        )
        .route(
            "/agents/{id}/memories/summarize",
            post(summarize_agent_memory),
        )
        .route(
            "/memory/process-embeddings",
            post(process_memory_embeddings),
        )
        .route("/memory/dead-letter", get(list_dead_letter_embeddings))
        .route(
            "/memory/dead-letter/{memory_id}/requeue",
            post(requeue_dead_letter_embedding),
        )
        .route("/ws/events", get(ws_events))
        .route("/ws/relationships", get(ws_relationships))
        .layer(from_fn_with_state(app_state.clone(), auth_middleware))
        .layer(from_fn_with_state(app_state.clone(), rate_limit_middleware))
        .layer(cors)
        .with_state(app_state);

    let socket_addr = config.socket_addr();
    let listener = tokio::net::TcpListener::bind(socket_addr)
        .await
        .with_context(|| format!("failed to bind API listener on {socket_addr}"))?;

    let shutdown_token = cancellation.clone();
    runtime.spawn("http_server", async move {
        tracing::info!(address = %socket_addr, "api server listening");
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            shutdown_token.cancelled().await;
        })
        .await
        .map_err(anyhow::Error::from)
    });

    let heartbeat_token = cancellation.clone();
    let service_name = config.common.service_name.clone();
    runtime.spawn("api_heartbeat", async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            tokio::select! {
                _ = heartbeat_token.cancelled() => {
                    tracing::info!("api heartbeat worker received shutdown");
                    break;
                }
                _ = interval.tick() => {
                    tracing::debug!(service = %service_name, "api heartbeat");
                }
            }
        }
        Ok(())
    });

    let event_bridge_repository = repository.clone();
    let event_bridge_hub = event_hub.clone();
    let event_bridge_interval = config.event_bridge_interval;
    let event_bridge_batch_size = config.event_bridge_batch_size;
    let event_bridge_token = cancellation.clone();
    runtime.spawn("event_bridge_worker", async move {
        let mut cursor: Option<i64> = None;
        let mut interval = tokio::time::interval(event_bridge_interval);
        loop {
            tokio::select! {
                _ = event_bridge_token.cancelled() => {
                    tracing::info!("event bridge worker received shutdown");
                    break;
                }
                _ = interval.tick() => {
                    if cursor.is_none() {
                        match event_bridge_repository.latest_event_id(None).await {
                            Ok(latest) => {
                                cursor = Some(latest.unwrap_or(0));
                            }
                            Err(error) => {
                                tracing::error!(error = %error, "event bridge failed to initialize cursor");
                            }
                        }
                        continue;
                    }

                    let mut tail_cursor = cursor.unwrap_or(0);
                    loop {
                        let records = match event_bridge_repository
                            .list_agent_events_after_id(None, tail_cursor, event_bridge_batch_size)
                            .await
                        {
                            Ok(records) => records,
                            Err(error) => {
                                tracing::error!(error = %error, after_id = tail_cursor, "event bridge failed to read new events");
                                break;
                            }
                        };

                        if records.is_empty() {
                            break;
                        }

                        let batch_len = records.len();
                        for record in records {
                            tail_cursor = tail_cursor.max(record.id);
                            let relationship_update = map_relationship_update_event(&record);
                            event_bridge_hub.publish(WsServerEvent::EventAppended {
                                item: map_event_record(&record),
                            });
                            if let Some(edge) = relationship_update {
                                event_bridge_hub.publish(WsServerEvent::RelationshipUpdated { edge });
                            }
                        }

                        if batch_len < event_bridge_batch_size as usize {
                            break;
                        }
                    }

                    cursor = Some(tail_cursor);
                }
            }
        }
        Ok(())
    });

    runtime.run_until_shutdown().await
}
