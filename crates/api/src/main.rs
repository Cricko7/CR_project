use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::extract::{
    Path, Query, State,
    ws::{Message, WebSocket, WebSocketUpgrade},
};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sim_backend::agent_core::{
    AgentCoreRepository, AgentEventRecord, AgentTickExecutionStatus, AgentTickOrchestrator,
    AgentTickOrchestratorOutcome, InterventionRecord, MessageRecord, NewAgentEvent,
    NewIntervention, NewMessage, RelationshipRecord,
};
use sim_backend::app::config::ApiConfig;
use sim_backend::app::observability::init_tracing;
use sim_backend::app::runtime::ServiceRuntime;
use sim_backend::infrastructure::gemini::GeminiClient;
use sim_backend::infrastructure::gemini_embedding::GeminiEmbeddingClient;
use sim_backend::infrastructure::postgres::{
    PostgresAgentCoreRepository, PostgresMemoryRepository, ensure_ready,
};
use sim_backend::infrastructure::qdrant::QdrantVectorStore;
use sim_backend::llm::LlmPort;
use sim_backend::memory::{
    MemoryEntryRecord, MemoryRecallItem, MemoryRepository, MemoryService, MemoryVectorStore,
    SimpleHashEmbedder, TextEmbedder,
};
use tokio::sync::broadcast;
use uuid::Uuid;

const EVENT_HUB_CAPACITY: usize = 512;
const DEFAULT_WS_SNAPSHOT_LIMIT: u32 = 20;
const DEFAULT_RECALL_TOP_K: u32 = 8;
const DEFAULT_RELATIONSHIP_GRAPH_LIMIT: u32 = 200;
const DEFAULT_INSPECTOR_LIMIT: u32 = 20;
const DEFAULT_INTERVENTION_LIMIT: u32 = 50;

#[derive(Clone)]
struct ApiState {
    service_name: String,
    repository: Arc<dyn AgentCoreRepository>,
    orchestrator: AgentTickOrchestrator,
    memory_service: Arc<MemoryService>,
    memory_defaults: MemoryRuntimeDefaults,
    event_hub: ApiEventHub,
}

#[derive(Clone)]
struct MemoryRuntimeDefaults {
    summary_max_active: u32,
    summary_batch_size: u32,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    service: String,
}

#[derive(Deserialize)]
struct TriggerTickRequest {
    tick_id: Option<String>,
}

#[derive(Serialize)]
struct TriggerTickResponse {
    outcome: &'static str,
    agent_id: Uuid,
    tick_id: Option<String>,
    event_id: Option<i64>,
    mood_label: Option<String>,
    valence: Option<f32>,
    arousal: Option<f32>,
}

#[derive(Deserialize)]
struct EventsQuery {
    agent_id: Option<Uuid>,
    limit: Option<u32>,
    after_id: Option<i64>,
}

#[derive(Serialize)]
struct EventsResponse {
    items: Vec<EventItemResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_after_id: Option<i64>,
}

#[derive(Serialize, Clone)]
struct EventItemResponse {
    id: i64,
    agent_id: Option<Uuid>,
    event_type: String,
    description: String,
    payload: String,
    occurred_at: String,
}

#[derive(Serialize)]
struct AgentStateResponse {
    agent_id: Uuid,
    mood_label: String,
    valence: f32,
    arousal: f32,
    updated_at: String,
}

#[derive(Deserialize)]
struct AgentInspectorQuery {
    events_limit: Option<u32>,
    messages_limit: Option<u32>,
    relationships_limit: Option<u32>,
    memories_limit: Option<u32>,
    recall_query: Option<String>,
    recall_top_k: Option<u32>,
}

#[derive(Serialize)]
struct AgentInspectorResponse {
    agent: AgentInspectorAgentResponse,
    state: Option<AgentStateResponse>,
    recent_events: Vec<EventItemResponse>,
    recent_messages: Vec<MessageItemResponse>,
    recent_relationships: Vec<RelationshipItemResponse>,
    recent_memories: Vec<InspectorMemoryItemResponse>,
    recall: Option<AgentInspectorRecallResponse>,
    summary: AgentInspectorSummaryResponse,
}

#[derive(Serialize)]
struct AgentInspectorAgentResponse {
    id: Uuid,
    name: String,
    avatar_url: Option<String>,
    personality_json: Value,
    created_at: String,
}

#[derive(Serialize)]
struct InspectorMemoryItemResponse {
    memory_id: i64,
    content: String,
    summary: Option<String>,
    importance: f32,
    is_summary: bool,
    embedding_status: String,
    created_at: String,
}

#[derive(Serialize)]
struct AgentInspectorRecallResponse {
    query: String,
    top_k: u32,
    items: Vec<RecallItemResponse>,
}

#[derive(Serialize)]
struct AgentInspectorSummaryResponse {
    events_count: usize,
    messages_count: usize,
    relationships_count: usize,
    memories_count: usize,
}

#[derive(Serialize, Clone)]
struct ApiErrorResponse {
    error: &'static str,
    message: String,
}

#[derive(Deserialize)]
struct AppendMemoryRequest {
    content: String,
    importance: Option<f32>,
}

#[derive(Serialize)]
struct AppendMemoryResponse {
    memory_id: i64,
    embedding_status: String,
}

#[derive(Deserialize)]
struct RecallQuery {
    query: String,
    top_k: Option<u32>,
}

#[derive(Serialize)]
struct RecallResponse {
    items: Vec<RecallItemResponse>,
}

#[derive(Serialize)]
struct RecallItemResponse {
    memory_id: i64,
    score: f32,
    content: String,
    summary: Option<String>,
    importance: f32,
    created_at: String,
}

#[derive(Deserialize)]
struct SummarizeMemoryRequest {
    max_active: Option<u32>,
    batch_size: Option<u32>,
}

#[derive(Serialize)]
struct SummarizeMemoryResponse {
    created_summary: bool,
    source_count: u32,
    summary_entry_id: Option<i64>,
}

#[derive(Deserialize)]
struct ProcessEmbeddingsRequest {
    limit: Option<u32>,
}

#[derive(Serialize)]
struct ProcessEmbeddingsResponse {
    processed: u32,
    succeeded: u32,
    failed: u32,
    retried: u32,
    dead_lettered: u32,
}

#[derive(Deserialize)]
struct DeadLetterQuery {
    limit: Option<u32>,
}

#[derive(Serialize)]
struct DeadLetterEmbeddingsResponse {
    items: Vec<DeadLetterEmbeddingItemResponse>,
}

#[derive(Serialize)]
struct DeadLetterEmbeddingItemResponse {
    memory_id: i64,
    agent_id: Uuid,
    content: String,
    summary: Option<String>,
    importance: f32,
    created_at: String,
    embedding_status: String,
}

#[derive(Serialize)]
struct RequeueDeadLetterResponse {
    memory_id: i64,
    requeued: bool,
}

#[derive(Deserialize)]
struct CreateInterventionRequest {
    admin_user_id: String,
    action: InterventionActionRequest,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum InterventionActionRequest {
    TriggerTick {
        agent_id: Uuid,
        tick_id: Option<String>,
    },
    AppendMemory {
        agent_id: Uuid,
        content: String,
        importance: Option<f32>,
    },
    SendMessage {
        sender_agent_id: Uuid,
        receiver_agent_id: Uuid,
        content: String,
    },
    AppendEvent {
        agent_id: Option<Uuid>,
        event_type: String,
        description: String,
        payload_json: Option<Value>,
    },
}

impl InterventionActionRequest {
    fn action_type(&self) -> &'static str {
        match self {
            Self::TriggerTick { .. } => "trigger_tick",
            Self::AppendMemory { .. } => "append_memory",
            Self::SendMessage { .. } => "send_message",
            Self::AppendEvent { .. } => "append_event",
        }
    }

    fn payload_json(&self) -> Value {
        match self {
            Self::TriggerTick { agent_id, tick_id } => json!({
                "agent_id": agent_id,
                "tick_id": tick_id,
            }),
            Self::AppendMemory {
                agent_id,
                content,
                importance,
            } => json!({
                "agent_id": agent_id,
                "content": content,
                "importance": importance,
            }),
            Self::SendMessage {
                sender_agent_id,
                receiver_agent_id,
                content,
            } => json!({
                "sender_agent_id": sender_agent_id,
                "receiver_agent_id": receiver_agent_id,
                "content": content,
            }),
            Self::AppendEvent {
                agent_id,
                event_type,
                description,
                payload_json,
            } => json!({
                "agent_id": agent_id,
                "event_type": event_type,
                "description": description,
                "payload_json": payload_json,
            }),
        }
    }
}

#[derive(Deserialize)]
struct InterventionListQuery {
    limit: Option<u32>,
}

#[derive(Serialize)]
struct InterventionsResponse {
    items: Vec<InterventionItemResponse>,
}

#[derive(Serialize)]
struct CreateInterventionResponse {
    intervention: InterventionItemResponse,
    effect: InterventionEffectResponse,
}

#[derive(Serialize)]
struct InterventionItemResponse {
    id: i64,
    admin_user_id: String,
    action_type: String,
    payload_json: Value,
    result_status: String,
    created_at: String,
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum InterventionEffectResponse {
    Tick {
        agent_id: Uuid,
        outcome: String,
        tick_id: Option<String>,
        event_id: Option<i64>,
        mood_label: Option<String>,
        valence: Option<f32>,
        arousal: Option<f32>,
    },
    Memory {
        agent_id: Uuid,
        memory_id: i64,
        embedding_status: String,
    },
    Message {
        message_id: i64,
        status: String,
    },
    Event {
        event_id: i64,
        event_type: String,
    },
}

#[derive(Deserialize)]
struct CreateMessageRequest {
    sender_agent_id: Uuid,
    content: String,
}

#[derive(Serialize)]
struct CreateMessageResponse {
    message_id: i64,
    status: String,
}

#[derive(Deserialize)]
struct MessageListQuery {
    limit: Option<u32>,
}

#[derive(Serialize)]
struct AgentMessagesResponse {
    items: Vec<MessageItemResponse>,
}

#[derive(Serialize)]
struct MessageItemResponse {
    id: i64,
    sender_type: String,
    sender_id: Option<Uuid>,
    receiver_agent_id: Uuid,
    content: String,
    status: String,
    created_at: String,
}

#[derive(Deserialize)]
struct RelationshipListQuery {
    limit: Option<u32>,
}

#[derive(Deserialize)]
struct RelationshipGraphQuery {
    agent_id: Option<Uuid>,
    limit_edges: Option<u32>,
}

#[derive(Serialize)]
struct AgentRelationshipsResponse {
    items: Vec<RelationshipItemResponse>,
}

#[derive(Serialize, Clone)]
struct RelationshipItemResponse {
    id: i64,
    agent_a: Uuid,
    agent_b: Uuid,
    affinity_score: f32,
    history_summary: String,
    last_interaction_at: Option<String>,
    created_at: String,
}

#[derive(Serialize, Clone)]
struct RelationshipGraphNodeResponse {
    agent_id: Uuid,
    name: String,
    avatar_url: Option<String>,
}

#[derive(Serialize, Clone)]
struct RelationshipGraphResponse {
    nodes: Vec<RelationshipGraphNodeResponse>,
    edges: Vec<RelationshipItemResponse>,
}

#[derive(Deserialize)]
struct WsEventsQuery {
    agent_id: Option<Uuid>,
    snapshot_limit: Option<u32>,
}

#[derive(Deserialize)]
struct WsRelationshipsQuery {
    agent_id: Option<Uuid>,
    snapshot_limit: Option<u32>,
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WsServerEvent {
    Snapshot {
        items: Vec<EventItemResponse>,
    },
    TickSkipped {
        agent_id: Uuid,
        reason: String,
        tick_id: Option<String>,
    },
    EventAppended {
        item: EventItemResponse,
    },
    RelationshipUpdated {
        edge: RelationshipItemResponse,
    },
    Error {
        message: String,
    },
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WsRelationshipServerEvent {
    Snapshot { graph: RelationshipGraphResponse },
    EdgeUpdated { edge: RelationshipItemResponse },
    Error { message: String },
}

#[derive(Clone)]
struct ApiEventHub {
    sender: broadcast::Sender<WsServerEvent>,
}

impl ApiEventHub {
    fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    fn publish(&self, event: WsServerEvent) {
        let _ = self.sender.send(event);
    }

    fn subscribe(&self) -> broadcast::Receiver<WsServerEvent> {
        self.sender.subscribe()
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = ApiConfig::from_env().context("failed to load API config")?;
    init_tracing(&config.common.service_name, &config.common.log_level)?;
    let db_pool = ensure_ready(&config.database)
        .await
        .context("database startup check failed")?;

    let gemini_client: Option<Arc<dyn LlmPort>> = match config.gemini.clone() {
        Some(gemini_config) => {
            tracing::info!(model = %gemini_config.model, "gemini client configured");
            Some(Arc::new(GeminiClient::new(gemini_config)?) as Arc<dyn LlmPort>)
        }
        None => {
            tracing::warn!("GEMINI_API_KEY is not set; api runs with deterministic tick fallback");
            None
        }
    };

    let repository: Arc<dyn AgentCoreRepository> =
        Arc::new(PostgresAgentCoreRepository::new(db_pool.clone()));
    let orchestrator =
        AgentTickOrchestrator::new(repository.clone()).with_optional_llm(gemini_client.clone());

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
        gemini_client,
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
    let app = Router::new()
        .route("/health", get(health))
        .route("/livez", get(health))
        .route("/agents/{id}/ticks", post(trigger_agent_tick))
        .route("/agents/{id}/state", get(get_agent_state))
        .route("/agents/{id}/inspector", get(get_agent_inspector))
        .route("/events", get(list_events))
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
        .with_state(state);

    let socket_addr = config.socket_addr();
    let listener = tokio::net::TcpListener::bind(socket_addr)
        .await
        .with_context(|| format!("failed to bind API listener on {socket_addr}"))?;

    let shutdown_token = cancellation.clone();
    runtime.spawn("http_server", async move {
        tracing::info!(address = %socket_addr, "api server listening");
        axum::serve(listener, app)
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

async fn health(State(state): State<ApiState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: state.service_name,
    })
}

async fn trigger_agent_tick(
    State(state): State<ApiState>,
    Path(agent_id): Path<Uuid>,
    payload: Option<Json<TriggerTickRequest>>,
) -> Result<(StatusCode, Json<TriggerTickResponse>), (StatusCode, Json<ApiErrorResponse>)> {
    let tick_id = payload.and_then(|Json(body)| body.tick_id);
    let outcome = state
        .orchestrator
        .run_agent_tick(agent_id, tick_id.clone())
        .await
        .map_err(|error| {
            internal_error("tick_failed", format!("failed to run agent tick: {error}"))
        })?;

    match outcome {
        AgentTickOrchestratorOutcome::Executed(result) => match result.status {
            AgentTickExecutionStatus::Applied => {
                if let Err(error) = state
                    .memory_service
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
                    tracing::error!(agent_id = %result.agent_id, error = %error, "failed to persist episodic memory from tick");
                }

                Ok((
                    StatusCode::OK,
                    Json(TriggerTickResponse {
                        outcome: "applied",
                        agent_id: result.agent_id,
                        tick_id: Some(result.tick_id),
                        event_id: result.event_id,
                        mood_label: Some(result.mood_label),
                        valence: Some(result.valence),
                        arousal: Some(result.arousal),
                    }),
                ))
            }
            AgentTickExecutionStatus::AgentMissing => Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResponse {
                    error: "agent_not_found",
                    message: format!("agent `{}` does not exist", result.agent_id),
                }),
            )),
        },
        AgentTickOrchestratorOutcome::SkippedBusy => {
            state.event_hub.publish(WsServerEvent::TickSkipped {
                agent_id,
                reason: "busy".to_owned(),
                tick_id: tick_id.clone(),
            });
            Ok((
                StatusCode::CONFLICT,
                Json(TriggerTickResponse {
                    outcome: "skipped_busy",
                    agent_id,
                    tick_id,
                    event_id: None,
                    mood_label: None,
                    valence: None,
                    arousal: None,
                }),
            ))
        }
        AgentTickOrchestratorOutcome::SkippedDuplicate => {
            state.event_hub.publish(WsServerEvent::TickSkipped {
                agent_id,
                reason: "duplicate".to_owned(),
                tick_id: tick_id.clone(),
            });
            Ok((
                StatusCode::CONFLICT,
                Json(TriggerTickResponse {
                    outcome: "skipped_duplicate",
                    agent_id,
                    tick_id,
                    event_id: None,
                    mood_label: None,
                    valence: None,
                    arousal: None,
                }),
            ))
        }
    }
}

async fn get_agent_state(
    State(state): State<ApiState>,
    Path(agent_id): Path<Uuid>,
) -> Result<Json<AgentStateResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let Some(record) = state
        .repository
        .get_agent_state(agent_id)
        .await
        .map_err(|error| {
            internal_error(
                "state_read_failed",
                format!("failed to read agent state: {error}"),
            )
        })?
    else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse {
                error: "state_not_found",
                message: format!("state for agent `{agent_id}` was not found"),
            }),
        ));
    };

    Ok(Json(AgentStateResponse {
        agent_id: record.agent_id,
        mood_label: record.mood_label,
        valence: record.valence,
        arousal: record.arousal,
        updated_at: record.updated_at.to_rfc3339(),
    }))
}

async fn get_agent_inspector(
    State(state): State<ApiState>,
    Path(agent_id): Path<Uuid>,
    Query(query): Query<AgentInspectorQuery>,
) -> Result<Json<AgentInspectorResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let events_limit = query
        .events_limit
        .unwrap_or(DEFAULT_INSPECTOR_LIMIT)
        .clamp(1, 200);
    let messages_limit = query
        .messages_limit
        .unwrap_or(DEFAULT_INSPECTOR_LIMIT)
        .clamp(1, 200);
    let relationships_limit = query
        .relationships_limit
        .unwrap_or(DEFAULT_INSPECTOR_LIMIT)
        .clamp(1, 200);
    let memories_limit = query
        .memories_limit
        .unwrap_or(DEFAULT_INSPECTOR_LIMIT)
        .clamp(1, 200);
    let recall_query = query
        .recall_query
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let recall_top_k = query
        .recall_top_k
        .unwrap_or(DEFAULT_RECALL_TOP_K)
        .clamp(1, 50);

    let repository = state.repository.clone();
    let memory_service = state.memory_service.clone();

    let (
        agent_result,
        state_result,
        events_result,
        messages_result,
        relationships_result,
        memories_result,
    ) = tokio::join!(
        repository.get_agent(agent_id),
        repository.get_agent_state(agent_id),
        repository.list_agent_events(Some(agent_id), events_limit),
        repository.list_agent_messages(agent_id, messages_limit),
        repository.list_agent_relationships(agent_id, relationships_limit),
        memory_service.list_recent_memories(agent_id, memories_limit),
    );

    let Some(agent) = agent_result.map_err(|error| {
        internal_error(
            "inspector_read_failed",
            format!("failed to read agent profile: {error}"),
        )
    })?
    else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse {
                error: "agent_not_found",
                message: format!("agent `{agent_id}` does not exist"),
            }),
        ));
    };

    let state_item = state_result
        .map_err(|error| {
            internal_error(
                "inspector_read_failed",
                format!("failed to read agent state: {error}"),
            )
        })?
        .map(|record| AgentStateResponse {
            agent_id: record.agent_id,
            mood_label: record.mood_label,
            valence: record.valence,
            arousal: record.arousal,
            updated_at: record.updated_at.to_rfc3339(),
        });

    let events = events_result.map_err(|error| {
        internal_error(
            "inspector_read_failed",
            format!("failed to read agent events: {error}"),
        )
    })?;
    let messages = messages_result.map_err(|error| {
        internal_error(
            "inspector_read_failed",
            format!("failed to read agent messages: {error}"),
        )
    })?;
    let relationships = relationships_result.map_err(|error| {
        internal_error(
            "inspector_read_failed",
            format!("failed to read agent relationships: {error}"),
        )
    })?;
    let memories = memories_result.map_err(|error| {
        internal_error(
            "inspector_read_failed",
            format!("failed to read agent memories: {error}"),
        )
    })?;

    let recall = if let Some(recall_query) = recall_query {
        let recalled = state
            .memory_service
            .recall(agent_id, &recall_query, recall_top_k)
            .await
            .map_err(|error| {
                internal_error(
                    "inspector_recall_failed",
                    format!("failed to run inspector memory recall: {error}"),
                )
            })?;
        Some(AgentInspectorRecallResponse {
            query: recall_query,
            top_k: recall_top_k,
            items: recalled.into_iter().map(map_recall_item).collect(),
        })
    } else {
        None
    };

    let recent_events: Vec<EventItemResponse> = events.iter().map(map_event_record).collect();
    let recent_messages: Vec<MessageItemResponse> =
        messages.into_iter().map(map_message_record).collect();
    let recent_relationships: Vec<RelationshipItemResponse> = relationships
        .into_iter()
        .map(map_relationship_record)
        .collect();
    let recent_memories: Vec<InspectorMemoryItemResponse> =
        memories.into_iter().map(map_inspector_memory).collect();

    Ok(Json(AgentInspectorResponse {
        agent: AgentInspectorAgentResponse {
            id: agent.id,
            name: agent.name,
            avatar_url: agent.avatar_url,
            personality_json: agent.personality_json,
            created_at: agent.created_at.to_rfc3339(),
        },
        state: state_item,
        summary: AgentInspectorSummaryResponse {
            events_count: recent_events.len(),
            messages_count: recent_messages.len(),
            relationships_count: recent_relationships.len(),
            memories_count: recent_memories.len(),
        },
        recent_events,
        recent_messages,
        recent_relationships,
        recent_memories,
        recall,
    }))
}

async fn create_intervention(
    State(state): State<ApiState>,
    Json(payload): Json<CreateInterventionRequest>,
) -> Result<(StatusCode, Json<CreateInterventionResponse>), (StatusCode, Json<ApiErrorResponse>)> {
    let admin_user_id = payload.admin_user_id.trim().to_owned();
    if admin_user_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_admin_user_id",
                message: "admin_user_id must not be empty".to_owned(),
            }),
        ));
    }

    let action_type = payload.action.action_type().to_owned();
    let action_payload = payload.action.payload_json();

    let effect = match apply_intervention_action(&state, &payload.action).await {
        Ok(effect) => effect,
        Err((status, Json(error_body))) => {
            let failed_payload = json!({
                "action": action_payload,
                "error": {
                    "code": error_body.error,
                    "message": error_body.message,
                }
            });
            if let Err(store_error) = state
                .repository
                .append_intervention(&NewIntervention {
                    admin_user_id,
                    action_type,
                    payload_json: failed_payload,
                    result_status: "failed".to_owned(),
                })
                .await
            {
                tracing::error!(error = %store_error, "failed to persist failed intervention record");
            }
            return Err((status, Json(error_body)));
        }
    };

    let record = state
        .repository
        .append_intervention(&NewIntervention {
            admin_user_id,
            action_type,
            payload_json: json!({
                "action": action_payload,
                "effect": effect.clone(),
            }),
            result_status: "applied".to_owned(),
        })
        .await
        .map_err(|error| {
            internal_error(
                "intervention_store_failed",
                format!("failed to store intervention: {error}"),
            )
        })?;

    Ok((
        StatusCode::OK,
        Json(CreateInterventionResponse {
            intervention: map_intervention_record(record),
            effect,
        }),
    ))
}

async fn list_interventions(
    State(state): State<ApiState>,
    Query(query): Query<InterventionListQuery>,
) -> Result<Json<InterventionsResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let limit = query
        .limit
        .unwrap_or(DEFAULT_INTERVENTION_LIMIT)
        .clamp(1, 200);
    let items = state
        .repository
        .list_interventions(limit)
        .await
        .map_err(|error| {
            internal_error(
                "intervention_list_failed",
                format!("failed to list interventions: {error}"),
            )
        })?
        .into_iter()
        .map(map_intervention_record)
        .collect();

    Ok(Json(InterventionsResponse { items }))
}

async fn list_events(
    State(state): State<ApiState>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<EventsResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let records = if let Some(after_id) = query.after_id {
        state
            .repository
            .list_agent_events_after_id(query.agent_id, after_id, limit)
            .await
            .map_err(|error| {
                internal_error(
                    "events_read_failed",
                    format!("failed to read events after id: {error}"),
                )
            })?
    } else {
        state
            .repository
            .list_agent_events(query.agent_id, limit)
            .await
            .map_err(|error| {
                internal_error(
                    "events_read_failed",
                    format!("failed to read events: {error}"),
                )
            })?
    };

    let next_after_id = if let Some(after_id) = query.after_id {
        Some(
            records
                .iter()
                .map(|record| record.id)
                .max()
                .unwrap_or(after_id),
        )
    } else {
        None
    };

    let items = records.iter().map(map_event_record).collect();

    Ok(Json(EventsResponse {
        items,
        next_after_id,
    }))
}

async fn append_agent_memory(
    State(state): State<ApiState>,
    Path(agent_id): Path<Uuid>,
    Json(payload): Json<AppendMemoryRequest>,
) -> Result<(StatusCode, Json<AppendMemoryResponse>), (StatusCode, Json<ApiErrorResponse>)> {
    if payload.content.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_content",
                message: "memory content must not be empty".to_owned(),
            }),
        ));
    }

    let memory = state
        .memory_service
        .append_memory(agent_id, payload.content, payload.importance.unwrap_or(0.6))
        .await
        .map_err(|error| {
            internal_error(
                "memory_append_failed",
                format!("failed to append memory: {error}"),
            )
        })?;

    Ok((
        StatusCode::CREATED,
        Json(AppendMemoryResponse {
            memory_id: memory.id,
            embedding_status: memory.embedding_status,
        }),
    ))
}

async fn send_agent_message(
    State(state): State<ApiState>,
    Path(receiver_agent_id): Path<Uuid>,
    Json(payload): Json<CreateMessageRequest>,
) -> Result<(StatusCode, Json<CreateMessageResponse>), (StatusCode, Json<ApiErrorResponse>)> {
    let message = enqueue_agent_message(
        state.repository.as_ref(),
        payload.sender_agent_id,
        receiver_agent_id,
        payload.content,
    )
    .await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(CreateMessageResponse {
            message_id: message.id,
            status: message.status,
        }),
    ))
}

async fn list_agent_messages(
    State(state): State<ApiState>,
    Path(agent_id): Path<Uuid>,
    Query(query): Query<MessageListQuery>,
) -> Result<Json<AgentMessagesResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let items = state
        .repository
        .list_agent_messages(agent_id, limit)
        .await
        .map_err(|error| {
            internal_error(
                "message_list_failed",
                format!("failed to list agent messages: {error}"),
            )
        })?
        .into_iter()
        .map(map_message_record)
        .collect();

    Ok(Json(AgentMessagesResponse { items }))
}

async fn list_agent_relationships(
    State(state): State<ApiState>,
    Path(agent_id): Path<Uuid>,
    Query(query): Query<RelationshipListQuery>,
) -> Result<Json<AgentRelationshipsResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let items = state
        .repository
        .list_agent_relationships(agent_id, limit)
        .await
        .map_err(|error| {
            internal_error(
                "relationship_list_failed",
                format!("failed to list agent relationships: {error}"),
            )
        })?
        .into_iter()
        .map(map_relationship_record)
        .collect();

    Ok(Json(AgentRelationshipsResponse { items }))
}

async fn get_relationship_graph(
    State(state): State<ApiState>,
    Query(query): Query<RelationshipGraphQuery>,
) -> Result<Json<RelationshipGraphResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let limit_edges = query
        .limit_edges
        .unwrap_or(DEFAULT_RELATIONSHIP_GRAPH_LIMIT)
        .clamp(1, 500);

    let graph = build_relationship_graph(&state, query.agent_id, limit_edges)
        .await
        .map_err(|error| {
            internal_error(
                "relationship_graph_failed",
                format!("failed to build relationship graph: {error}"),
            )
        })?;

    Ok(Json(graph))
}

async fn recall_agent_memory(
    State(state): State<ApiState>,
    Path(agent_id): Path<Uuid>,
    Query(query): Query<RecallQuery>,
) -> Result<Json<RecallResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    if query.query.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_query",
                message: "recall query must not be empty".to_owned(),
            }),
        ));
    }

    let top_k = query.top_k.unwrap_or(DEFAULT_RECALL_TOP_K).clamp(1, 50);
    let recalled = state
        .memory_service
        .recall(agent_id, &query.query, top_k)
        .await
        .map_err(|error| {
            internal_error(
                "memory_recall_failed",
                format!("failed to recall memory: {error}"),
            )
        })?;

    let items = recalled.into_iter().map(map_recall_item).collect();
    Ok(Json(RecallResponse { items }))
}

async fn summarize_agent_memory(
    State(state): State<ApiState>,
    Path(agent_id): Path<Uuid>,
    payload: Option<Json<SummarizeMemoryRequest>>,
) -> Result<Json<SummarizeMemoryResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let body = payload.map(|Json(value)| value);
    let max_active = body
        .as_ref()
        .and_then(|v| v.max_active)
        .unwrap_or(state.memory_defaults.summary_max_active);
    let batch_size = body
        .as_ref()
        .and_then(|v| v.batch_size)
        .unwrap_or(state.memory_defaults.summary_batch_size);

    let result = state
        .memory_service
        .summarize_overflow(agent_id, max_active, batch_size)
        .await
        .map_err(|error| {
            internal_error(
                "memory_summarize_failed",
                format!("failed to summarize memory overflow: {error}"),
            )
        })?;

    Ok(Json(SummarizeMemoryResponse {
        created_summary: result.created_summary,
        source_count: result.source_count,
        summary_entry_id: result.summary_entry_id,
    }))
}

async fn process_memory_embeddings(
    State(state): State<ApiState>,
    payload: Option<Json<ProcessEmbeddingsRequest>>,
) -> Result<Json<ProcessEmbeddingsResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let limit = payload
        .map(|Json(value)| value.limit.unwrap_or(50))
        .unwrap_or(50)
        .clamp(1, 200);
    let summary = state
        .memory_service
        .process_pending_embeddings(limit)
        .await
        .map_err(|error| {
            internal_error(
                "memory_embedding_failed",
                format!("failed to process memory embeddings: {error}"),
            )
        })?;

    Ok(Json(ProcessEmbeddingsResponse {
        processed: summary.processed,
        succeeded: summary.succeeded,
        failed: summary.failed,
        retried: summary.retried,
        dead_lettered: summary.dead_lettered,
    }))
}

async fn list_dead_letter_embeddings(
    State(state): State<ApiState>,
    Query(query): Query<DeadLetterQuery>,
) -> Result<Json<DeadLetterEmbeddingsResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let entries = state
        .memory_service
        .list_dead_letter_embeddings(limit)
        .await
        .map_err(|error| {
            internal_error(
                "memory_dead_letter_read_failed",
                format!("failed to list dead-letter memory embeddings: {error}"),
            )
        })?;

    Ok(Json(DeadLetterEmbeddingsResponse {
        items: entries.into_iter().map(map_dead_letter_memory).collect(),
    }))
}

async fn requeue_dead_letter_embedding(
    State(state): State<ApiState>,
    Path(memory_id): Path<i64>,
) -> Result<Json<RequeueDeadLetterResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let requeued = state
        .memory_service
        .requeue_dead_letter_embedding(memory_id)
        .await
        .map_err(|error| {
            internal_error(
                "memory_dead_letter_requeue_failed",
                format!("failed to requeue dead-letter memory embedding: {error}"),
            )
        })?;

    Ok(Json(RequeueDeadLetterResponse {
        memory_id,
        requeued,
    }))
}

async fn apply_intervention_action(
    state: &ApiState,
    action: &InterventionActionRequest,
) -> Result<InterventionEffectResponse, (StatusCode, Json<ApiErrorResponse>)> {
    match action {
        InterventionActionRequest::TriggerTick { agent_id, tick_id } => {
            let outcome = state
                .orchestrator
                .run_agent_tick(*agent_id, tick_id.clone())
                .await
                .map_err(|error| {
                    internal_error(
                        "intervention_apply_failed",
                        format!("failed to run intervention tick: {error}"),
                    )
                })?;

            match outcome {
                AgentTickOrchestratorOutcome::Executed(result) => match result.status {
                    AgentTickExecutionStatus::Applied => {
                        if let Err(error) = state
                            .memory_service
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
                            tracing::error!(agent_id = %result.agent_id, error = %error, "failed to persist episodic memory from intervention tick");
                        }

                        Ok(InterventionEffectResponse::Tick {
                            agent_id: result.agent_id,
                            outcome: "applied".to_owned(),
                            tick_id: Some(result.tick_id),
                            event_id: result.event_id,
                            mood_label: Some(result.mood_label),
                            valence: Some(result.valence),
                            arousal: Some(result.arousal),
                        })
                    }
                    AgentTickExecutionStatus::AgentMissing => Err((
                        StatusCode::NOT_FOUND,
                        Json(ApiErrorResponse {
                            error: "agent_not_found",
                            message: format!("agent `{}` does not exist", result.agent_id),
                        }),
                    )),
                },
                AgentTickOrchestratorOutcome::SkippedBusy => {
                    state.event_hub.publish(WsServerEvent::TickSkipped {
                        agent_id: *agent_id,
                        reason: "busy".to_owned(),
                        tick_id: tick_id.clone(),
                    });
                    Ok(InterventionEffectResponse::Tick {
                        agent_id: *agent_id,
                        outcome: "skipped_busy".to_owned(),
                        tick_id: tick_id.clone(),
                        event_id: None,
                        mood_label: None,
                        valence: None,
                        arousal: None,
                    })
                }
                AgentTickOrchestratorOutcome::SkippedDuplicate => {
                    state.event_hub.publish(WsServerEvent::TickSkipped {
                        agent_id: *agent_id,
                        reason: "duplicate".to_owned(),
                        tick_id: tick_id.clone(),
                    });
                    Ok(InterventionEffectResponse::Tick {
                        agent_id: *agent_id,
                        outcome: "skipped_duplicate".to_owned(),
                        tick_id: tick_id.clone(),
                        event_id: None,
                        mood_label: None,
                        valence: None,
                        arousal: None,
                    })
                }
            }
        }
        InterventionActionRequest::AppendMemory {
            agent_id,
            content,
            importance,
        } => {
            if content.trim().is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResponse {
                        error: "invalid_content",
                        message: "memory content must not be empty".to_owned(),
                    }),
                ));
            }

            let agent_exists = state
                .repository
                .get_agent(*agent_id)
                .await
                .map_err(|error| {
                    internal_error(
                        "intervention_apply_failed",
                        format!("failed to load intervention target agent: {error}"),
                    )
                })?
                .is_some();
            if !agent_exists {
                return Err((
                    StatusCode::NOT_FOUND,
                    Json(ApiErrorResponse {
                        error: "agent_not_found",
                        message: format!("agent `{agent_id}` does not exist"),
                    }),
                ));
            }

            let memory = state
                .memory_service
                .append_memory(*agent_id, content.clone(), importance.unwrap_or(0.6))
                .await
                .map_err(|error| {
                    internal_error(
                        "intervention_apply_failed",
                        format!("failed to append memory intervention: {error}"),
                    )
                })?;

            Ok(InterventionEffectResponse::Memory {
                agent_id: *agent_id,
                memory_id: memory.id,
                embedding_status: memory.embedding_status,
            })
        }
        InterventionActionRequest::SendMessage {
            sender_agent_id,
            receiver_agent_id,
            content,
        } => {
            let message = enqueue_agent_message(
                state.repository.as_ref(),
                *sender_agent_id,
                *receiver_agent_id,
                content.clone(),
            )
            .await?;
            Ok(InterventionEffectResponse::Message {
                message_id: message.id,
                status: message.status,
            })
        }
        InterventionActionRequest::AppendEvent {
            agent_id,
            event_type,
            description,
            payload_json,
        } => {
            if event_type.trim().is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResponse {
                        error: "invalid_event_type",
                        message: "event_type must not be empty".to_owned(),
                    }),
                ));
            }
            if description.trim().is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResponse {
                        error: "invalid_description",
                        message: "description must not be empty".to_owned(),
                    }),
                ));
            }
            if let Some(agent_id) = agent_id {
                let exists = state
                    .repository
                    .get_agent(*agent_id)
                    .await
                    .map_err(|error| {
                        internal_error(
                            "intervention_apply_failed",
                            format!("failed to load event target agent: {error}"),
                        )
                    })?
                    .is_some();
                if !exists {
                    return Err((
                        StatusCode::NOT_FOUND,
                        Json(ApiErrorResponse {
                            error: "agent_not_found",
                            message: format!("agent `{agent_id}` does not exist"),
                        }),
                    ));
                }
            }

            let event = state
                .repository
                .append_agent_event(&NewAgentEvent {
                    agent_id: *agent_id,
                    event_type: event_type.trim().to_owned(),
                    description: description.trim().to_owned(),
                    payload_json: payload_json.clone().unwrap_or_else(|| json!({})),
                })
                .await
                .map_err(|error| {
                    internal_error(
                        "intervention_apply_failed",
                        format!("failed to append intervention event: {error}"),
                    )
                })?;

            Ok(InterventionEffectResponse::Event {
                event_id: event.id,
                event_type: event.event_type,
            })
        }
    }
}

async fn enqueue_agent_message(
    repository: &dyn AgentCoreRepository,
    sender_agent_id: Uuid,
    receiver_agent_id: Uuid,
    content: String,
) -> Result<MessageRecord, (StatusCode, Json<ApiErrorResponse>)> {
    if content.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_content",
                message: "message content must not be empty".to_owned(),
            }),
        ));
    }
    if sender_agent_id == receiver_agent_id {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: "invalid_sender",
                message: "sender and receiver must be different agents".to_owned(),
            }),
        ));
    }

    let sender_exists = repository
        .get_agent(sender_agent_id)
        .await
        .map_err(|error| {
            internal_error(
                "message_send_failed",
                format!("failed to load sender agent: {error}"),
            )
        })?
        .is_some();
    if !sender_exists {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse {
                error: "sender_not_found",
                message: format!("sender agent `{sender_agent_id}` does not exist"),
            }),
        ));
    }

    let receiver_exists = repository
        .get_agent(receiver_agent_id)
        .await
        .map_err(|error| {
            internal_error(
                "message_send_failed",
                format!("failed to load receiver agent: {error}"),
            )
        })?
        .is_some();
    if !receiver_exists {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse {
                error: "receiver_not_found",
                message: format!("receiver agent `{receiver_agent_id}` does not exist"),
            }),
        ));
    }

    repository
        .enqueue_message(&NewMessage {
            sender_type: "agent".to_owned(),
            sender_id: Some(sender_agent_id),
            receiver_agent_id,
            content: content.trim().to_owned(),
        })
        .await
        .map_err(|error| {
            internal_error(
                "message_send_failed",
                format!("failed to enqueue agent message: {error}"),
            )
        })
}

async fn ws_events(
    State(state): State<ApiState>,
    Query(query): Query<WsEventsQuery>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_events_session(socket, state, query))
}

async fn ws_relationships(
    State(state): State<ApiState>,
    Query(query): Query<WsRelationshipsQuery>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_relationships_session(socket, state, query))
}

async fn ws_events_session(mut socket: WebSocket, state: ApiState, query: WsEventsQuery) {
    let snapshot_limit = query
        .snapshot_limit
        .unwrap_or(DEFAULT_WS_SNAPSHOT_LIMIT)
        .clamp(1, 200);
    let snapshot = state
        .repository
        .list_agent_events(query.agent_id, snapshot_limit)
        .await;

    match snapshot {
        Ok(records) => {
            let items = records.iter().map(map_event_record).collect();
            let event = WsServerEvent::Snapshot { items };
            if !send_ws_event(&mut socket, &event).await {
                return;
            }
        }
        Err(error) => {
            let event = WsServerEvent::Error {
                message: format!("failed to load snapshot: {error}"),
            };
            let _ = send_ws_event(&mut socket, &event).await;
            return;
        }
    }

    let mut receiver = state.event_hub.subscribe();
    loop {
        match receiver.recv().await {
            Ok(event) => {
                if matches!(event, WsServerEvent::RelationshipUpdated { .. }) {
                    continue;
                }
                if !ws_event_matches_agent(&event, query.agent_id) {
                    continue;
                }
                if !send_ws_event(&mut socket, &event).await {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!(
                    skipped_messages = skipped,
                    "disconnecting lagging websocket client"
                );
                let _ = send_ws_event(
                    &mut socket,
                    &WsServerEvent::Error {
                        message: format!(
                            "stream lagged by {skipped} events; reconnect for fresh state"
                        ),
                    },
                )
                .await;
                break;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

async fn ws_relationships_session(
    mut socket: WebSocket,
    state: ApiState,
    query: WsRelationshipsQuery,
) {
    let snapshot_limit = query
        .snapshot_limit
        .unwrap_or(DEFAULT_RELATIONSHIP_GRAPH_LIMIT)
        .clamp(1, 500);

    let snapshot = build_relationship_graph(&state, query.agent_id, snapshot_limit).await;
    match snapshot {
        Ok(graph) => {
            if !send_ws_relationship_event(
                &mut socket,
                &WsRelationshipServerEvent::Snapshot { graph },
            )
            .await
            {
                return;
            }
        }
        Err(error) => {
            let _ = send_ws_relationship_event(
                &mut socket,
                &WsRelationshipServerEvent::Error {
                    message: format!("failed to load relationship graph snapshot: {error}"),
                },
            )
            .await;
            return;
        }
    }

    let mut receiver = state.event_hub.subscribe();
    loop {
        match receiver.recv().await {
            Ok(WsServerEvent::RelationshipUpdated { edge }) => {
                if !relationship_edge_matches_agent(&edge, query.agent_id) {
                    continue;
                }
                if !send_ws_relationship_event(
                    &mut socket,
                    &WsRelationshipServerEvent::EdgeUpdated { edge },
                )
                .await
                {
                    break;
                }
            }
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!(
                    skipped_messages = skipped,
                    "disconnecting lagging relationship websocket client"
                );
                let _ = send_ws_relationship_event(
                    &mut socket,
                    &WsRelationshipServerEvent::Error {
                        message: format!(
                            "relationship stream lagged by {skipped} events; reconnect for fresh state"
                        ),
                    },
                )
                .await;
                break;
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

async fn send_ws_event(socket: &mut WebSocket, event: &WsServerEvent) -> bool {
    let payload = match serde_json::to_string(event) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::error!(error = %error, "failed to serialize websocket event");
            return false;
        }
    };

    match socket.send(Message::Text(payload.into())).await {
        Ok(_) => true,
        Err(error) => {
            tracing::debug!(error = %error, "websocket send failed");
            false
        }
    }
}

async fn send_ws_relationship_event(
    socket: &mut WebSocket,
    event: &WsRelationshipServerEvent,
) -> bool {
    let payload = match serde_json::to_string(event) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::error!(error = %error, "failed to serialize relationship websocket event");
            return false;
        }
    };

    match socket.send(Message::Text(payload.into())).await {
        Ok(_) => true,
        Err(error) => {
            tracing::debug!(error = %error, "relationship websocket send failed");
            false
        }
    }
}

fn ws_event_matches_agent(event: &WsServerEvent, agent_id: Option<Uuid>) -> bool {
    let Some(agent_id) = agent_id else {
        return true;
    };

    match event {
        WsServerEvent::Snapshot { .. } | WsServerEvent::Error { .. } => true,
        WsServerEvent::TickSkipped {
            agent_id: event_agent_id,
            ..
        } => *event_agent_id == agent_id,
        WsServerEvent::EventAppended { item } => item.agent_id == Some(agent_id),
        WsServerEvent::RelationshipUpdated { edge } => {
            edge.agent_a == agent_id || edge.agent_b == agent_id
        }
    }
}

fn relationship_edge_matches_agent(
    edge: &RelationshipItemResponse,
    agent_id: Option<Uuid>,
) -> bool {
    match agent_id {
        Some(agent_id) => edge.agent_a == agent_id || edge.agent_b == agent_id,
        None => true,
    }
}

fn map_event_record(record: &AgentEventRecord) -> EventItemResponse {
    EventItemResponse {
        id: record.id,
        agent_id: record.agent_id,
        event_type: record.event_type.clone(),
        description: record.description.clone(),
        payload: record.payload_json.to_string(),
        occurred_at: record.occurred_at.to_rfc3339(),
    }
}

fn map_relationship_update_event(record: &AgentEventRecord) -> Option<RelationshipItemResponse> {
    if record.event_type != "agent.relationship.updated" {
        return None;
    }

    let payload = record.payload_json.as_object()?;
    let id = payload.get("relationship_id")?.as_i64()?;
    let agent_a = Uuid::parse_str(payload.get("agent_a")?.as_str()?).ok()?;
    let agent_b = Uuid::parse_str(payload.get("agent_b")?.as_str()?).ok()?;
    let affinity_score = payload.get("affinity_score")?.as_f64()? as f32;
    let history_summary = payload
        .get("history_summary")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_owned();
    let last_interaction_at = payload
        .get("last_interaction_at")
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    let created_at = payload
        .get("created_at")
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| record.occurred_at.to_rfc3339());

    Some(RelationshipItemResponse {
        id,
        agent_a,
        agent_b,
        affinity_score,
        history_summary,
        last_interaction_at,
        created_at,
    })
}

async fn build_relationship_graph(
    state: &ApiState,
    agent_id: Option<Uuid>,
    limit_edges: u32,
) -> anyhow::Result<RelationshipGraphResponse> {
    let edges_raw = match agent_id {
        Some(agent_id) => {
            state
                .repository
                .list_agent_relationships(agent_id, limit_edges)
                .await?
        }
        None => state.repository.list_relationships(limit_edges).await?,
    };

    let edges: Vec<RelationshipItemResponse> = edges_raw
        .iter()
        .cloned()
        .map(map_relationship_record)
        .collect();

    let mut participant_ids = HashSet::new();
    for edge in &edges {
        participant_ids.insert(edge.agent_a);
        participant_ids.insert(edge.agent_b);
    }

    let mut nodes = Vec::with_capacity(participant_ids.len());
    for participant_id in participant_ids {
        let agent = state.repository.get_agent(participant_id).await?;
        let (name, avatar_url) = match agent {
            Some(agent) => (agent.name, agent.avatar_url),
            None => (format!("agent-{participant_id}"), None),
        };
        nodes.push(RelationshipGraphNodeResponse {
            agent_id: participant_id,
            name,
            avatar_url,
        });
    }
    nodes.sort_by(|left, right| left.name.cmp(&right.name));

    Ok(RelationshipGraphResponse { nodes, edges })
}

fn map_recall_item(item: MemoryRecallItem) -> RecallItemResponse {
    RecallItemResponse {
        memory_id: item.memory.id,
        score: item.score,
        content: item.memory.content,
        summary: item.memory.summary,
        importance: item.memory.importance,
        created_at: item.memory.created_at.to_rfc3339(),
    }
}

fn map_dead_letter_memory(memory: MemoryEntryRecord) -> DeadLetterEmbeddingItemResponse {
    DeadLetterEmbeddingItemResponse {
        memory_id: memory.id,
        agent_id: memory.agent_id,
        content: memory.content,
        summary: memory.summary,
        importance: memory.importance,
        created_at: memory.created_at.to_rfc3339(),
        embedding_status: memory.embedding_status,
    }
}

fn map_inspector_memory(memory: MemoryEntryRecord) -> InspectorMemoryItemResponse {
    InspectorMemoryItemResponse {
        memory_id: memory.id,
        content: memory.content,
        summary: memory.summary,
        importance: memory.importance,
        is_summary: memory.is_summary,
        embedding_status: memory.embedding_status,
        created_at: memory.created_at.to_rfc3339(),
    }
}

fn map_intervention_record(record: InterventionRecord) -> InterventionItemResponse {
    InterventionItemResponse {
        id: record.id,
        admin_user_id: record.admin_user_id,
        action_type: record.action_type,
        payload_json: record.payload_json,
        result_status: record.result_status,
        created_at: record.created_at.to_rfc3339(),
    }
}

fn map_message_record(message: MessageRecord) -> MessageItemResponse {
    MessageItemResponse {
        id: message.id,
        sender_type: message.sender_type,
        sender_id: message.sender_id,
        receiver_agent_id: message.receiver_agent_id,
        content: message.content,
        status: message.status,
        created_at: message.created_at.to_rfc3339(),
    }
}

fn map_relationship_record(record: RelationshipRecord) -> RelationshipItemResponse {
    RelationshipItemResponse {
        id: record.id,
        agent_a: record.agent_a,
        agent_b: record.agent_b,
        affinity_score: record.affinity_score,
        history_summary: record.history_summary,
        last_interaction_at: record.last_interaction_at.map(|value| value.to_rfc3339()),
        created_at: record.created_at.to_rfc3339(),
    }
}

fn internal_error(error: &'static str, message: String) -> (StatusCode, Json<ApiErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiErrorResponse { error, message }),
    )
}
