use super::*;

#[derive(Clone)]
pub(super) struct ApiState {
    pub(super) service_name: String,
    pub(super) repository: Arc<dyn AgentCoreRepository>,
    pub(super) orchestrator: AgentTickOrchestrator,
    pub(super) memory_service: Arc<MemoryService>,
    pub(super) memory_defaults: MemoryRuntimeDefaults,
    pub(super) event_hub: ApiEventHub,
}

#[derive(Clone)]
pub(super) struct AuthState {
    pub(super) manager: Arc<AuthManager>,
    pub(super) repository: Arc<PostgresAuthRepository>,
}

#[derive(Clone)]
pub(super) struct RateLimitState {
    pub(super) limiter: Arc<IpRateLimiter>,
}

#[derive(Clone)]
pub(super) struct AppState {
    pub(super) api: ApiState,
    pub(super) auth: AuthState,
    pub(super) rate_limit: RateLimitState,
}

impl axum::extract::FromRef<AppState> for ApiState {
    fn from_ref(state: &AppState) -> Self {
        state.api.clone()
    }
}

impl axum::extract::FromRef<AppState> for AuthState {
    fn from_ref(state: &AppState) -> Self {
        state.auth.clone()
    }
}

impl axum::extract::FromRef<AppState> for RateLimitState {
    fn from_ref(state: &AppState) -> Self {
        state.rate_limit.clone()
    }
}

#[derive(Clone)]
pub(super) struct MemoryRuntimeDefaults {
    pub(super) summary_max_active: u32,
    pub(super) summary_batch_size: u32,
}
