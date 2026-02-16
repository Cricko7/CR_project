use std::env;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use uuid::Uuid;

const DEFAULT_SERVICE_NAME: &str = "sim-backend";
const DEFAULT_LOG_LEVEL: &str = "info";
const DEFAULT_HOST: &str = "0.0.0.0";
const DEFAULT_API_PORT: u16 = 8080;
const DEFAULT_API_EVENT_BRIDGE_INTERVAL_MS: u64 = 500;
const DEFAULT_API_EVENT_BRIDGE_BATCH_SIZE: u32 = 128;
const DEFAULT_SHUTDOWN_TIMEOUT_MS: u64 = 15_000;
const DEFAULT_WORKER_TICK_INTERVAL_MS: u64 = 1_000;
const DEFAULT_WORKER_TICK_CONCURRENCY: u32 = 8;
const DEFAULT_MOOD_DECAY_INTERVAL_MS: u64 = 5_000;
const DEFAULT_MOOD_DECAY_STEP: f32 = 0.06;
const DEFAULT_WORKER_MESSAGE_INTERVAL_MS: u64 = 1_000;
const DEFAULT_WORKER_MESSAGE_BATCH_SIZE: u32 = 32;
const DEFAULT_DATABASE_MAX_CONNECTIONS: u32 = 10;
const DEFAULT_DATABASE_CONNECT_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_DATABASE_ACQUIRE_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_DATABASE_IDLE_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_DATABASE_MAX_LIFETIME_MS: u64 = 300_000;
const DEFAULT_DATABASE_RUN_MIGRATIONS: bool = false;
const DEFAULT_GEMINI_MODEL: &str = "gemini-2.0-flash";
const DEFAULT_GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com";
const DEFAULT_GEMINI_TIMEOUT_MS: u64 = 15_000;
const DEFAULT_GEMINI_MAX_RETRIES: u32 = 2;
const DEFAULT_GEMINI_RETRY_BACKOFF_MS: u64 = 300;
const DEFAULT_GEMINI_EMBED_MODEL: &str = "text-embedding-004";
const DEFAULT_QDRANT_URL: &str = "http://localhost:6333";
const DEFAULT_QDRANT_COLLECTION: &str = "agent_memories";
const DEFAULT_QDRANT_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_QDRANT_VECTOR_SIZE: u32 = 768;
const DEFAULT_MEMORY_EMBED_BATCH_SIZE: u32 = 32;
const DEFAULT_MEMORY_MAX_ACTIVE_PER_AGENT: u32 = 200;
const DEFAULT_MEMORY_SUMMARY_BATCH_SIZE: u32 = 20;
const DEFAULT_MEMORY_EMBED_INTERVAL_MS: u64 = 5_000;
const DEFAULT_MEMORY_SUMMARY_INTERVAL_MS: u64 = 30_000;

#[derive(Debug, Clone)]
pub struct CommonConfig {
    pub service_name: String,
    pub log_level: String,
    pub shutdown_timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub connect_timeout: Duration,
    pub acquire_timeout: Duration,
    pub idle_timeout: Duration,
    pub max_lifetime: Duration,
    pub run_migrations: bool,
}

#[derive(Debug, Clone)]
pub struct GeminiConfig {
    pub api_key: String,
    pub model: String,
    pub embedding_model: String,
    pub base_url: String,
    pub timeout: Duration,
    pub max_retries: u32,
    pub retry_backoff: Duration,
}

#[derive(Debug, Clone)]
pub struct QdrantConfig {
    pub url: String,
    pub api_key: Option<String>,
    pub collection: String,
    pub vector_size: u32,
    pub timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct MemoryConfig {
    pub embed_batch_size: u32,
    pub max_active_per_agent: u32,
    pub summary_batch_size: u32,
    pub embed_interval: Duration,
    pub summary_interval: Duration,
}

#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub common: CommonConfig,
    pub database: DatabaseConfig,
    pub qdrant: QdrantConfig,
    pub memory: MemoryConfig,
    pub gemini: Option<GeminiConfig>,
    pub host: IpAddr,
    pub port: u16,
    pub event_bridge_interval: Duration,
    pub event_bridge_batch_size: u32,
}

impl ApiConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let common = load_common_config(DEFAULT_SERVICE_NAME)?;
        let database = load_database_config()?;
        let qdrant = load_qdrant_config()?;
        let memory = load_memory_config()?;
        let gemini = load_gemini_config()?;
        let host_raw: String = parse_env("API_HOST", DEFAULT_HOST.to_owned())?;
        let host = host_raw
            .parse::<IpAddr>()
            .map_err(|error| ConfigError::parse("API_HOST", error.to_string()))?;
        let port = parse_env("API_PORT", DEFAULT_API_PORT)?;
        let event_bridge_interval_ms: u64 = parse_env(
            "API_EVENT_BRIDGE_INTERVAL_MS",
            DEFAULT_API_EVENT_BRIDGE_INTERVAL_MS,
        )?;
        let event_bridge_batch_size: u32 = parse_env(
            "API_EVENT_BRIDGE_BATCH_SIZE",
            DEFAULT_API_EVENT_BRIDGE_BATCH_SIZE,
        )?;
        if port == 0 {
            return Err(ConfigError::invalid("API_PORT", "must be greater than 0"));
        }
        if event_bridge_interval_ms == 0 {
            return Err(ConfigError::invalid(
                "API_EVENT_BRIDGE_INTERVAL_MS",
                "must be greater than 0",
            ));
        }
        if event_bridge_batch_size == 0 {
            return Err(ConfigError::invalid(
                "API_EVENT_BRIDGE_BATCH_SIZE",
                "must be greater than 0",
            ));
        }

        Ok(Self {
            common,
            database,
            qdrant,
            memory,
            gemini,
            host,
            port,
            event_bridge_interval: Duration::from_millis(event_bridge_interval_ms),
            event_bridge_batch_size,
        })
    }

    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.host, self.port)
    }
}

#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub common: CommonConfig,
    pub database: DatabaseConfig,
    pub qdrant: QdrantConfig,
    pub memory: MemoryConfig,
    pub gemini: Option<GeminiConfig>,
    pub agent_ids: Vec<Uuid>,
    pub tick_interval: Duration,
    pub tick_concurrency: u32,
    pub mood_decay_interval: Duration,
    pub mood_decay_step: f32,
    pub message_interval: Duration,
    pub message_batch_size: u32,
}

impl WorkerConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let common = load_common_config("sim-worker")?;
        let database = load_database_config()?;
        let qdrant = load_qdrant_config()?;
        let memory = load_memory_config()?;
        let gemini = load_gemini_config()?;
        let agent_ids = parse_uuid_list_env("WORKER_AGENT_IDS")?;
        let tick_interval_ms: u64 =
            parse_env("WORKER_TICK_INTERVAL_MS", DEFAULT_WORKER_TICK_INTERVAL_MS)?;
        let tick_concurrency: u32 =
            parse_env("WORKER_TICK_CONCURRENCY", DEFAULT_WORKER_TICK_CONCURRENCY)?;
        let mood_decay_interval_ms: u64 = parse_env(
            "WORKER_MOOD_DECAY_INTERVAL_MS",
            DEFAULT_MOOD_DECAY_INTERVAL_MS,
        )?;
        let mood_decay_step: f32 = parse_env("WORKER_MOOD_DECAY_STEP", DEFAULT_MOOD_DECAY_STEP)?;
        let message_interval_ms: u64 = parse_env(
            "WORKER_MESSAGE_INTERVAL_MS",
            DEFAULT_WORKER_MESSAGE_INTERVAL_MS,
        )?;
        let message_batch_size: u32 = parse_env(
            "WORKER_MESSAGE_BATCH_SIZE",
            DEFAULT_WORKER_MESSAGE_BATCH_SIZE,
        )?;

        if tick_interval_ms == 0 {
            return Err(ConfigError::invalid(
                "WORKER_TICK_INTERVAL_MS",
                "must be greater than 0",
            ));
        }
        if mood_decay_interval_ms == 0 {
            return Err(ConfigError::invalid(
                "WORKER_MOOD_DECAY_INTERVAL_MS",
                "must be greater than 0",
            ));
        }
        if tick_concurrency == 0 {
            return Err(ConfigError::invalid(
                "WORKER_TICK_CONCURRENCY",
                "must be greater than 0",
            ));
        }
        if !(0.0..=1.0).contains(&mood_decay_step) || mood_decay_step == 0.0 {
            return Err(ConfigError::invalid(
                "WORKER_MOOD_DECAY_STEP",
                "must be in range (0.0, 1.0]",
            ));
        }
        if message_interval_ms == 0 {
            return Err(ConfigError::invalid(
                "WORKER_MESSAGE_INTERVAL_MS",
                "must be greater than 0",
            ));
        }
        if message_batch_size == 0 {
            return Err(ConfigError::invalid(
                "WORKER_MESSAGE_BATCH_SIZE",
                "must be greater than 0",
            ));
        }

        Ok(Self {
            common,
            database,
            qdrant,
            memory,
            gemini,
            agent_ids,
            tick_interval: Duration::from_millis(tick_interval_ms),
            tick_concurrency,
            mood_decay_interval: Duration::from_millis(mood_decay_interval_ms),
            mood_decay_step,
            message_interval: Duration::from_millis(message_interval_ms),
            message_batch_size,
        })
    }
}

fn load_common_config(default_service_name: &str) -> Result<CommonConfig, ConfigError> {
    let service_name = env::var("SERVICE_NAME").unwrap_or_else(|_| default_service_name.to_owned());
    let service_name = service_name.trim().to_owned();
    if service_name.is_empty() {
        return Err(ConfigError::invalid("SERVICE_NAME", "must not be empty"));
    }

    let log_level = env::var("LOG_LEVEL")
        .unwrap_or_else(|_| DEFAULT_LOG_LEVEL.to_owned())
        .to_lowercase();
    validate_log_level(&log_level)?;

    let shutdown_timeout_ms: u64 = parse_env("SHUTDOWN_TIMEOUT_MS", DEFAULT_SHUTDOWN_TIMEOUT_MS)?;
    if shutdown_timeout_ms == 0 {
        return Err(ConfigError::invalid(
            "SHUTDOWN_TIMEOUT_MS",
            "must be greater than 0",
        ));
    }

    Ok(CommonConfig {
        service_name,
        log_level,
        shutdown_timeout: Duration::from_millis(shutdown_timeout_ms),
    })
}

fn load_database_config() -> Result<DatabaseConfig, ConfigError> {
    let url = env::var("DATABASE_URL")
        .map(|value| value.trim().to_owned())
        .unwrap_or_default();
    if url.is_empty() {
        return Err(ConfigError::invalid("DATABASE_URL", "must not be empty"));
    }

    let max_connections: u32 =
        parse_env("DATABASE_MAX_CONNECTIONS", DEFAULT_DATABASE_MAX_CONNECTIONS)?;
    if max_connections == 0 {
        return Err(ConfigError::invalid(
            "DATABASE_MAX_CONNECTIONS",
            "must be greater than 0",
        ));
    }

    let connect_timeout_ms: u64 = parse_env(
        "DATABASE_CONNECT_TIMEOUT_MS",
        DEFAULT_DATABASE_CONNECT_TIMEOUT_MS,
    )?;
    let acquire_timeout_ms: u64 = parse_env(
        "DATABASE_ACQUIRE_TIMEOUT_MS",
        DEFAULT_DATABASE_ACQUIRE_TIMEOUT_MS,
    )?;
    let idle_timeout_ms: u64 =
        parse_env("DATABASE_IDLE_TIMEOUT_MS", DEFAULT_DATABASE_IDLE_TIMEOUT_MS)?;
    let max_lifetime_ms: u64 =
        parse_env("DATABASE_MAX_LIFETIME_MS", DEFAULT_DATABASE_MAX_LIFETIME_MS)?;
    let run_migrations: bool =
        parse_env("DATABASE_RUN_MIGRATIONS", DEFAULT_DATABASE_RUN_MIGRATIONS)?;

    if connect_timeout_ms == 0 {
        return Err(ConfigError::invalid(
            "DATABASE_CONNECT_TIMEOUT_MS",
            "must be greater than 0",
        ));
    }
    if acquire_timeout_ms == 0 {
        return Err(ConfigError::invalid(
            "DATABASE_ACQUIRE_TIMEOUT_MS",
            "must be greater than 0",
        ));
    }
    if idle_timeout_ms == 0 {
        return Err(ConfigError::invalid(
            "DATABASE_IDLE_TIMEOUT_MS",
            "must be greater than 0",
        ));
    }
    if max_lifetime_ms == 0 {
        return Err(ConfigError::invalid(
            "DATABASE_MAX_LIFETIME_MS",
            "must be greater than 0",
        ));
    }

    Ok(DatabaseConfig {
        url,
        max_connections,
        connect_timeout: Duration::from_millis(connect_timeout_ms),
        acquire_timeout: Duration::from_millis(acquire_timeout_ms),
        idle_timeout: Duration::from_millis(idle_timeout_ms),
        max_lifetime: Duration::from_millis(max_lifetime_ms),
        run_migrations,
    })
}

fn validate_log_level(raw: &str) -> Result<(), ConfigError> {
    let valid = ["trace", "debug", "info", "warn", "error"];
    if valid.contains(&raw) {
        Ok(())
    } else {
        Err(ConfigError::invalid(
            "LOG_LEVEL",
            "must be one of trace|debug|info|warn|error",
        ))
    }
}

fn load_gemini_config() -> Result<Option<GeminiConfig>, ConfigError> {
    let api_key = env::var("GEMINI_API_KEY")
        .map(|value| value.trim().to_owned())
        .unwrap_or_default();
    if api_key.is_empty() {
        return Ok(None);
    }

    let model: String = parse_env("GEMINI_MODEL", DEFAULT_GEMINI_MODEL.to_owned())?;
    let embedding_model: String =
        parse_env("GEMINI_EMBED_MODEL", DEFAULT_GEMINI_EMBED_MODEL.to_owned())?;
    let base_url: String = parse_env("GEMINI_BASE_URL", DEFAULT_GEMINI_BASE_URL.to_owned())?;
    let timeout_ms: u64 = parse_env("GEMINI_TIMEOUT_MS", DEFAULT_GEMINI_TIMEOUT_MS)?;
    let max_retries: u32 = parse_env("GEMINI_MAX_RETRIES", DEFAULT_GEMINI_MAX_RETRIES)?;
    let retry_backoff_ms: u64 =
        parse_env("GEMINI_RETRY_BACKOFF_MS", DEFAULT_GEMINI_RETRY_BACKOFF_MS)?;

    if model.trim().is_empty() {
        return Err(ConfigError::invalid("GEMINI_MODEL", "must not be empty"));
    }
    if embedding_model.trim().is_empty() {
        return Err(ConfigError::invalid(
            "GEMINI_EMBED_MODEL",
            "must not be empty",
        ));
    }
    if base_url.trim().is_empty() {
        return Err(ConfigError::invalid("GEMINI_BASE_URL", "must not be empty"));
    }
    if timeout_ms == 0 {
        return Err(ConfigError::invalid(
            "GEMINI_TIMEOUT_MS",
            "must be greater than 0",
        ));
    }
    if retry_backoff_ms == 0 {
        return Err(ConfigError::invalid(
            "GEMINI_RETRY_BACKOFF_MS",
            "must be greater than 0",
        ));
    }

    Ok(Some(GeminiConfig {
        api_key,
        model,
        embedding_model,
        base_url,
        timeout: Duration::from_millis(timeout_ms),
        max_retries,
        retry_backoff: Duration::from_millis(retry_backoff_ms),
    }))
}

fn load_qdrant_config() -> Result<QdrantConfig, ConfigError> {
    let url: String = parse_env("QDRANT_URL", DEFAULT_QDRANT_URL.to_owned())?;
    let api_key_raw = env::var("QDRANT_API_KEY").unwrap_or_default();
    let api_key = if api_key_raw.trim().is_empty() {
        None
    } else {
        Some(api_key_raw.trim().to_owned())
    };
    let collection: String = parse_env("QDRANT_COLLECTION", DEFAULT_QDRANT_COLLECTION.to_owned())?;
    let timeout_ms: u64 = parse_env("QDRANT_TIMEOUT_MS", DEFAULT_QDRANT_TIMEOUT_MS)?;
    let vector_size: u32 = parse_env("QDRANT_VECTOR_SIZE", DEFAULT_QDRANT_VECTOR_SIZE)?;

    if url.trim().is_empty() {
        return Err(ConfigError::invalid("QDRANT_URL", "must not be empty"));
    }
    if collection.trim().is_empty() {
        return Err(ConfigError::invalid(
            "QDRANT_COLLECTION",
            "must not be empty",
        ));
    }
    if timeout_ms == 0 {
        return Err(ConfigError::invalid(
            "QDRANT_TIMEOUT_MS",
            "must be greater than 0",
        ));
    }
    if vector_size == 0 {
        return Err(ConfigError::invalid(
            "QDRANT_VECTOR_SIZE",
            "must be greater than 0",
        ));
    }

    Ok(QdrantConfig {
        url,
        api_key,
        collection,
        vector_size,
        timeout: Duration::from_millis(timeout_ms),
    })
}

fn load_memory_config() -> Result<MemoryConfig, ConfigError> {
    let embed_batch_size: u32 =
        parse_env("MEMORY_EMBED_BATCH_SIZE", DEFAULT_MEMORY_EMBED_BATCH_SIZE)?;
    let max_active_per_agent: u32 = parse_env(
        "MEMORY_MAX_ACTIVE_PER_AGENT",
        DEFAULT_MEMORY_MAX_ACTIVE_PER_AGENT,
    )?;
    let summary_batch_size: u32 = parse_env(
        "MEMORY_SUMMARY_BATCH_SIZE",
        DEFAULT_MEMORY_SUMMARY_BATCH_SIZE,
    )?;
    let embed_interval_ms: u64 =
        parse_env("MEMORY_EMBED_INTERVAL_MS", DEFAULT_MEMORY_EMBED_INTERVAL_MS)?;
    let summary_interval_ms: u64 = parse_env(
        "MEMORY_SUMMARY_INTERVAL_MS",
        DEFAULT_MEMORY_SUMMARY_INTERVAL_MS,
    )?;

    if embed_batch_size == 0 {
        return Err(ConfigError::invalid(
            "MEMORY_EMBED_BATCH_SIZE",
            "must be greater than 0",
        ));
    }
    if max_active_per_agent == 0 {
        return Err(ConfigError::invalid(
            "MEMORY_MAX_ACTIVE_PER_AGENT",
            "must be greater than 0",
        ));
    }
    if summary_batch_size == 0 {
        return Err(ConfigError::invalid(
            "MEMORY_SUMMARY_BATCH_SIZE",
            "must be greater than 0",
        ));
    }
    if embed_interval_ms == 0 {
        return Err(ConfigError::invalid(
            "MEMORY_EMBED_INTERVAL_MS",
            "must be greater than 0",
        ));
    }
    if summary_interval_ms == 0 {
        return Err(ConfigError::invalid(
            "MEMORY_SUMMARY_INTERVAL_MS",
            "must be greater than 0",
        ));
    }

    Ok(MemoryConfig {
        embed_batch_size,
        max_active_per_agent,
        summary_batch_size,
        embed_interval: Duration::from_millis(embed_interval_ms),
        summary_interval: Duration::from_millis(summary_interval_ms),
    })
}

fn parse_uuid_list_env(key: &'static str) -> Result<Vec<Uuid>, ConfigError> {
    let raw = env::var(key).unwrap_or_default();
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }

    raw.split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(|token| {
            Uuid::parse_str(token).map_err(|error| ConfigError::parse(key, error.to_string()))
        })
        .collect()
}

fn parse_env<T>(key: &'static str, default: T) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    <T as std::str::FromStr>::Err: Display,
{
    match env::var(key) {
        Ok(value) => value
            .parse::<T>()
            .map_err(|error| ConfigError::parse(key, error.to_string())),
        Err(_) => Ok(default),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    key: &'static str,
    message: String,
}

impl ConfigError {
    fn parse(key: &'static str, message: String) -> Self {
        Self { key, message }
    }

    fn invalid(key: &'static str, message: &'static str) -> Self {
        Self {
            key,
            message: message.to_owned(),
        }
    }
}

impl Display for ConfigError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid config {}: {}", self.key, self.message)
    }
}

impl Error for ConfigError {}
