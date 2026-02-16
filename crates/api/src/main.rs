use std::time::Duration;

use anyhow::Context;
use axum::extract::State;
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use serde::Serialize;
use sim_backend::app::config::ApiConfig;
use sim_backend::app::observability::init_tracing;
use sim_backend::app::runtime::ServiceRuntime;

#[derive(Clone)]
struct ApiState {
    service_name: String,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    service: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = ApiConfig::from_env().context("failed to load API config")?;
    init_tracing(&config.common.service_name, &config.common.log_level)?;

    let mut runtime = ServiceRuntime::new(
        config.common.service_name.clone(),
        config.common.shutdown_timeout,
    );
    let cancellation = runtime.cancellation_token();

    let state = ApiState {
        service_name: config.common.service_name.clone(),
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/livez", get(health))
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
