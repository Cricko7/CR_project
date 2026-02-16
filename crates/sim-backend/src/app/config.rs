use std::env;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

const DEFAULT_SERVICE_NAME: &str = "sim-backend";
const DEFAULT_LOG_LEVEL: &str = "info";
const DEFAULT_HOST: &str = "0.0.0.0";
const DEFAULT_API_PORT: u16 = 8080;
const DEFAULT_SHUTDOWN_TIMEOUT_MS: u64 = 15_000;
const DEFAULT_WORKER_TICK_INTERVAL_MS: u64 = 1_000;
const DEFAULT_MOOD_DECAY_INTERVAL_MS: u64 = 5_000;
const DEFAULT_DATABASE_MAX_CONNECTIONS: u32 = 10;
const DEFAULT_DATABASE_CONNECT_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_DATABASE_ACQUIRE_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_DATABASE_IDLE_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_DATABASE_MAX_LIFETIME_MS: u64 = 300_000;
const DEFAULT_DATABASE_RUN_MIGRATIONS: bool = false;

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
pub struct ApiConfig {
    pub common: CommonConfig,
    pub database: DatabaseConfig,
    pub host: IpAddr,
    pub port: u16,
}

impl ApiConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let common = load_common_config(DEFAULT_SERVICE_NAME)?;
        let database = load_database_config()?;
        let host_raw: String = parse_env("API_HOST", DEFAULT_HOST.to_owned())?;
        let host = host_raw
            .parse::<IpAddr>()
            .map_err(|error| ConfigError::parse("API_HOST", error.to_string()))?;
        let port = parse_env("API_PORT", DEFAULT_API_PORT)?;
        if port == 0 {
            return Err(ConfigError::invalid("API_PORT", "must be greater than 0"));
        }

        Ok(Self {
            common,
            database,
            host,
            port,
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
    pub tick_interval: Duration,
    pub mood_decay_interval: Duration,
}

impl WorkerConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let common = load_common_config("sim-worker")?;
        let database = load_database_config()?;
        let tick_interval_ms: u64 =
            parse_env("WORKER_TICK_INTERVAL_MS", DEFAULT_WORKER_TICK_INTERVAL_MS)?;
        let mood_decay_interval_ms: u64 =
            parse_env("WORKER_MOOD_DECAY_INTERVAL_MS", DEFAULT_MOOD_DECAY_INTERVAL_MS)?;

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

        Ok(Self {
            common,
            database,
            tick_interval: Duration::from_millis(tick_interval_ms),
            mood_decay_interval: Duration::from_millis(mood_decay_interval_ms),
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

    let shutdown_timeout_ms: u64 =
        parse_env("SHUTDOWN_TIMEOUT_MS", DEFAULT_SHUTDOWN_TIMEOUT_MS)?;
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

    let max_connections: u32 = parse_env("DATABASE_MAX_CONNECTIONS", DEFAULT_DATABASE_MAX_CONNECTIONS)?;
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
    let run_migrations: bool = parse_env(
        "DATABASE_RUN_MIGRATIONS",
        DEFAULT_DATABASE_RUN_MIGRATIONS,
    )?;

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
