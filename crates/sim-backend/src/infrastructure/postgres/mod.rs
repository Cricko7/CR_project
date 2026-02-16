mod agent_core_repository;
mod memory_repository;

use std::time::Duration;

use anyhow::{Context, Result};
pub use agent_core_repository::PostgresAgentCoreRepository;
pub use memory_repository::PostgresMemoryRepository;
use sqlx::postgres::{PgPool, PgPoolOptions};

use crate::app::config::DatabaseConfig;

const HEALTHCHECK_QUERY: &str = "SELECT 1";

pub async fn connect_pool(config: &DatabaseConfig) -> Result<PgPool> {
    tokio::time::timeout(
        config.connect_timeout,
        PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(config.acquire_timeout)
        .idle_timeout(Some(config.idle_timeout))
        .max_lifetime(Some(config.max_lifetime))
        .connect(&config.url),
    )
    .await
    .context("timed out while connecting to PostgreSQL")?
    .with_context(|| "failed to connect to PostgreSQL")
}

pub async fn verify_connectivity(pool: &PgPool) -> Result<()> {
    sqlx::query(HEALTHCHECK_QUERY)
        .execute(pool)
        .await
        .context("database connectivity check failed")?;
    Ok(())
}

pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("failed to run database migrations")
}

pub async fn ensure_ready(config: &DatabaseConfig) -> Result<PgPool> {
    let pool = connect_pool(config).await?;
    verify_connectivity(&pool).await?;
    if config.run_migrations {
        run_migrations(&pool).await?;
    }
    Ok(pool)
}

pub fn default_retry_backoff(attempt: u32) -> Duration {
    let capped_attempt = attempt.min(6);
    Duration::from_millis(250 * 2u64.pow(capped_attempt))
}
