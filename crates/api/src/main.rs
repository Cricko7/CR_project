use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::extract::{
    ws::{Message, WebSocket, WebSocketUpgrade},
    Path, Query, State,
};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sim_backend::agent_core::{
    AgentCoreRepository, AgentEventRecord, AgentTickExecutionStatus, AgentTickOrchestrator,
    AgentTickOrchestratorOutcome,
};
use sim_backend::app::config::ApiConfig;
use sim_backend::app::observability::init_tracing;
use sim_backend::app::runtime::ServiceRuntime;
use sim_backend::infrastructure::gemini::GeminiClient;
use sim_backend::infrastructure::postgres::{PostgresAgentCoreRepository, ensure_ready};
use sim_backend::llm::LlmPort;
use tokio::sync::broadcast;
use uuid::Uuid;

const EVENT_HUB_CAPACITY: usize = 512;
const DEFAULT_WS_SNAPSHOT_LIMIT: u32 = 20;

#[derive(Clone)]
struct ApiState {
    service_name: String,
    repository: Arc<dyn AgentCoreRepository>,
    orchestrator: AgentTickOrchestrator,
    event_hub: ApiEventHub,
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
}

#[derive(Serialize)]
struct EventsResponse {
    items: Vec<EventItemResponse>,
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

#[derive(Serialize)]
struct ApiErrorResponse {
    error: &'static str,
    message: String,
}

#[derive(Deserialize)]
struct WsEventsQuery {
    agent_id: Option<Uuid>,
    snapshot_limit: Option<u32>,
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WsServerEvent {
    Snapshot {
        items: Vec<EventItemResponse>,
    },
    TickApplied {
        agent_id: Uuid,
        tick_id: String,
        event_id: Option<i64>,
        mood_label: String,
        valence: f32,
        arousal: f32,
    },
    TickSkipped {
        agent_id: Uuid,
        reason: String,
        tick_id: Option<String>,
    },
    Error {
        message: String,
    },
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
        Arc::new(PostgresAgentCoreRepository::new(db_pool));
    let orchestrator = AgentTickOrchestrator::new(repository.clone()).with_optional_llm(gemini_client);

    let mut runtime = ServiceRuntime::new(
        config.common.service_name.clone(),
        config.common.shutdown_timeout,
    );
    let cancellation = runtime.cancellation_token();

    let state = ApiState {
        service_name: config.common.service_name.clone(),
        repository,
        orchestrator,
        event_hub: ApiEventHub::new(EVENT_HUB_CAPACITY),
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/livez", get(health))
        .route("/agents/{id}/ticks", post(trigger_agent_tick))
        .route("/agents/{id}/state", get(get_agent_state))
        .route("/events", get(list_events))
        .route("/ws/events", get(ws_events))
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
            internal_error(
                "tick_failed",
                format!("failed to run agent tick: {error}"),
            )
        })?;

    match outcome {
        AgentTickOrchestratorOutcome::Executed(result) => match result.status {
            AgentTickExecutionStatus::Applied => {
                state.event_hub.publish(WsServerEvent::TickApplied {
                    agent_id: result.agent_id,
                    tick_id: result.tick_id.clone(),
                    event_id: result.event_id,
                    mood_label: result.mood_label.clone(),
                    valence: result.valence,
                    arousal: result.arousal,
                });

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
        AgentTickOrchestratorOutcome::SkippedBusy => Ok((
            StatusCode::CONFLICT,
            {
                state.event_hub.publish(WsServerEvent::TickSkipped {
                    agent_id,
                    reason: "busy".to_owned(),
                    tick_id: tick_id.clone(),
                });
                Json(TriggerTickResponse {
                    outcome: "skipped_busy",
                    agent_id,
                    tick_id,
                    event_id: None,
                    mood_label: None,
                    valence: None,
                    arousal: None,
                })
            },
        )),
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
    let Some(record) = state.repository.get_agent_state(agent_id).await.map_err(|error| {
        internal_error(
            "state_read_failed",
            format!("failed to read agent state: {error}"),
        )
    })? else {
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

async fn list_events(
    State(state): State<ApiState>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<EventsResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let records = state
        .repository
        .list_agent_events(query.agent_id, limit)
        .await
        .map_err(|error| {
            internal_error(
                "events_read_failed",
                format!("failed to read events: {error}"),
            )
        })?;

    let items = records
        .into_iter()
        .map(map_event_record)
        .collect();

    Ok(Json(EventsResponse { items }))
}

async fn ws_events(
    State(state): State<ApiState>,
    Query(query): Query<WsEventsQuery>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_events_session(socket, state, query))
}

async fn ws_events_session(mut socket: WebSocket, state: ApiState, query: WsEventsQuery) {
    let snapshot_limit = query.snapshot_limit.unwrap_or(DEFAULT_WS_SNAPSHOT_LIMIT).clamp(1, 200);
    let snapshot = state
        .repository
        .list_agent_events(query.agent_id, snapshot_limit)
        .await;

    match snapshot {
        Ok(records) => {
            let items = records.into_iter().map(map_event_record).collect();
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

fn map_event_record(record: AgentEventRecord) -> EventItemResponse {
    EventItemResponse {
        id: record.id,
        agent_id: record.agent_id,
        event_type: record.event_type,
        description: record.description,
        payload: record.payload_json.to_string(),
        occurred_at: record.occurred_at.to_rfc3339(),
    }
}

fn internal_error(
    error: &'static str,
    message: String,
) -> (StatusCode, Json<ApiErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiErrorResponse { error, message }),
    )
}
