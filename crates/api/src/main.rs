use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sim_backend::agent_core::{
    AgentCoreRepository, AgentTickExecutionStatus, AgentTickOrchestrator,
    AgentTickOrchestratorOutcome,
};
use sim_backend::app::config::ApiConfig;
use sim_backend::app::observability::init_tracing;
use sim_backend::app::runtime::ServiceRuntime;
use sim_backend::infrastructure::postgres::{PostgresAgentCoreRepository, ensure_ready};
use uuid::Uuid;

#[derive(Clone)]
struct ApiState {
    service_name: String,
    repository: Arc<dyn AgentCoreRepository>,
    orchestrator: AgentTickOrchestrator,
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

#[derive(Serialize)]
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = ApiConfig::from_env().context("failed to load API config")?;
    init_tracing(&config.common.service_name, &config.common.log_level)?;
    let db_pool = ensure_ready(&config.database)
        .await
        .context("database startup check failed")?;
    let repository: Arc<dyn AgentCoreRepository> =
        Arc::new(PostgresAgentCoreRepository::new(db_pool));
    let orchestrator = AgentTickOrchestrator::new(repository.clone());

    let mut runtime = ServiceRuntime::new(
        config.common.service_name.clone(),
        config.common.shutdown_timeout,
    );
    let cancellation = runtime.cancellation_token();

    let state = ApiState {
        service_name: config.common.service_name.clone(),
        repository,
        orchestrator,
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/livez", get(health))
        .route("/agents/{id}/ticks", post(trigger_agent_tick))
        .route("/agents/{id}/state", get(get_agent_state))
        .route("/events", get(list_events))
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
            AgentTickExecutionStatus::Applied => Ok((
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
            )),
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
            Json(TriggerTickResponse {
                outcome: "skipped_busy",
                agent_id,
                tick_id,
                event_id: None,
                mood_label: None,
                valence: None,
                arousal: None,
            }),
        )),
        AgentTickOrchestratorOutcome::SkippedDuplicate => Ok((
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
        )),
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
        .map(|record| EventItemResponse {
            id: record.id,
            agent_id: record.agent_id,
            event_type: record.event_type,
            description: record.description,
            payload: record.payload_json.to_string(),
            occurred_at: record.occurred_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(EventsResponse { items }))
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
