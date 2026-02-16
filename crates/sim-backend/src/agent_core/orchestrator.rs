use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::agent_core::{
    AgentCoreRepository, AgentStateRecord, AgentTickRunner, NewAgentEvent, TickLeaseAcquireResult,
    TickRunOutcome,
};
use crate::llm::{LlmGenerateRequest, LlmPort};

const DEFAULT_TICK_HISTORY_PER_AGENT: usize = 256;
const DEFAULT_TICK_LEASE_TTL: Duration = Duration::from_secs(90);
const LLM_SUMMARY_MAX_CHARS: usize = 512;
const EVENT_DESCRIPTION_MAX_CHARS: usize = 180;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentTickExecutionStatus {
    Applied,
    AgentMissing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentTickExecutionResult {
    pub agent_id: Uuid,
    pub tick_id: String,
    pub status: AgentTickExecutionStatus,
    pub event_id: Option<i64>,
    pub action_summary: String,
    pub mood_label: String,
    pub valence: f32,
    pub arousal: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentTickOrchestratorOutcome {
    Executed(AgentTickExecutionResult),
    SkippedBusy,
    SkippedDuplicate,
}

#[derive(Clone)]
pub struct AgentTickOrchestrator {
    repository: Arc<dyn AgentCoreRepository>,
    tick_runner: AgentTickRunner,
    llm: Option<Arc<dyn LlmPort>>,
}

impl AgentTickOrchestrator {
    pub fn new(repository: Arc<dyn AgentCoreRepository>) -> Self {
        Self::with_dependencies(
            repository,
            AgentTickRunner::new(DEFAULT_TICK_HISTORY_PER_AGENT),
            None,
        )
    }

    pub fn with_tick_runner(
        repository: Arc<dyn AgentCoreRepository>,
        tick_runner: AgentTickRunner,
    ) -> Self {
        Self::with_dependencies(repository, tick_runner, None)
    }

    pub fn with_dependencies(
        repository: Arc<dyn AgentCoreRepository>,
        tick_runner: AgentTickRunner,
        llm: Option<Arc<dyn LlmPort>>,
    ) -> Self {
        Self {
            repository,
            tick_runner,
            llm,
        }
    }

    pub fn with_optional_llm(mut self, llm: Option<Arc<dyn LlmPort>>) -> Self {
        self.llm = llm;
        self
    }

    pub async fn run_agent_tick(
        &self,
        agent_id: Uuid,
        tick_id: Option<String>,
    ) -> Result<AgentTickOrchestratorOutcome> {
        let tick_id = tick_id.unwrap_or_else(|| Uuid::new_v4().to_string());
        if self
            .repository
            .has_completed_tick(agent_id, &tick_id)
            .await?
        {
            return Ok(AgentTickOrchestratorOutcome::SkippedDuplicate);
        }

        match self
            .repository
            .try_acquire_tick_lease(agent_id, &tick_id, DEFAULT_TICK_LEASE_TTL)
            .await?
        {
            TickLeaseAcquireResult::Acquired => {}
            TickLeaseAcquireResult::Busy => {
                return Ok(AgentTickOrchestratorOutcome::SkippedBusy);
            }
        }

        let repository = Arc::clone(&self.repository);
        let llm = self.llm.clone();
        let tick_id_for_closure = tick_id.clone();

        let tick_outcome = self
            .tick_runner
            .run_tick(&agent_id.to_string(), &tick_id, move || async move {
                execute_tick(repository.as_ref(), llm, agent_id, &tick_id_for_closure).await
            })
            .await;

        let release_result = self.repository.release_tick_lease(agent_id, &tick_id).await;
        if let Err(error) = release_result {
            tracing::error!(
                agent_id = %agent_id,
                tick_id,
                error = %error,
                "failed to release global tick lease"
            );
            return Err(error.context("failed to release global tick lease"));
        }

        if matches!(
            tick_outcome,
            TickRunOutcome::Executed(_) | TickRunOutcome::SkippedDuplicate
        ) {
            self.repository
                .record_completed_tick(agent_id, &tick_id)
                .await
                .context("failed to persist completed tick idempotency")?;
        }

        match tick_outcome {
            TickRunOutcome::Executed(result) => Ok(AgentTickOrchestratorOutcome::Executed(result?)),
            TickRunOutcome::SkippedBusy => Ok(AgentTickOrchestratorOutcome::SkippedBusy),
            TickRunOutcome::SkippedDuplicate => Ok(AgentTickOrchestratorOutcome::SkippedDuplicate),
        }
    }
}

async fn execute_tick(
    repository: &dyn AgentCoreRepository,
    llm: Option<Arc<dyn LlmPort>>,
    agent_id: Uuid,
    tick_id: &str,
) -> Result<AgentTickExecutionResult> {
    let Some(agent) = repository.get_agent(agent_id).await? else {
        return Ok(AgentTickExecutionResult {
            agent_id,
            tick_id: tick_id.to_owned(),
            status: AgentTickExecutionStatus::AgentMissing,
            event_id: None,
            action_summary: "agent not found".to_owned(),
            mood_label: "unknown".to_owned(),
            valence: 0.0,
            arousal: 0.0,
        });
    };

    let now = Utc::now();
    let current_state = repository.get_agent_state(agent_id).await?;

    let (next_valence, next_arousal, next_mood) = match current_state {
        Some(state) => (
            state.valence.clamp(-1.0, 1.0),
            state.arousal.clamp(-1.0, 1.0),
            state.mood_label,
        ),
        None => (0.0, 0.0, "neutral".to_owned()),
    };

    repository
        .upsert_agent_state(&AgentStateRecord {
            agent_id,
            valence: next_valence,
            arousal: next_arousal,
            mood_label: next_mood.clone(),
            updated_at: now,
        })
        .await?;

    let (action_summary, llm_used, llm_model, llm_error, llm_latency_ms) = generate_action_summary(
        llm.as_deref(),
        &agent.name,
        &agent.personality_json,
        tick_id,
        &next_mood,
        next_valence,
        next_arousal,
    )
    .await;

    let description = format!(
        "Agent `{}` executed tick `{}`: {}",
        agent.name,
        tick_id,
        trim_text(&action_summary, EVENT_DESCRIPTION_MAX_CHARS)
    );

    let event = repository
        .append_agent_event(&NewAgentEvent {
            agent_id: Some(agent_id),
            event_type: "agent.tick.executed".to_owned(),
            description,
            payload_json: json!({
                "agent_id": agent_id,
                "tick_id": tick_id,
                "mood_label": next_mood,
                "valence": next_valence,
                "arousal": next_arousal,
                "action_summary": action_summary,
                "llm": {
                    "configured": llm.is_some(),
                    "used": llm_used,
                    "model": llm_model,
                    "error": llm_error,
                    "latency_ms": llm_latency_ms,
                }
            }),
        })
        .await?;

    Ok(AgentTickExecutionResult {
        agent_id,
        tick_id: tick_id.to_owned(),
        status: AgentTickExecutionStatus::Applied,
        event_id: Some(event.id),
        action_summary,
        mood_label: next_mood,
        valence: next_valence,
        arousal: next_arousal,
    })
}

async fn generate_action_summary(
    llm: Option<&dyn LlmPort>,
    agent_name: &str,
    personality_json: &Value,
    tick_id: &str,
    mood_label: &str,
    valence: f32,
    arousal: f32,
) -> (String, bool, Option<String>, Option<String>, Option<u128>) {
    let fallback = fallback_summary(agent_name, mood_label);

    let Some(llm) = llm else {
        return (
            fallback,
            false,
            None,
            Some("llm_not_configured".to_owned()),
            None,
        );
    };

    let request = LlmGenerateRequest {
        system_prompt: Some(format!(
            "You are a planning core for autonomous AI agent `{agent_name}`. Personality JSON: {personality_json}. Return only 1-2 concise sentences: reflection and immediate next action."
        )),
        user_prompt: format!(
            "Tick: {tick_id}. Mood: {mood_label}. Valence: {valence:.2}. Arousal: {arousal:.2}. Generate reflection + next action."
        ),
        temperature: Some(0.5),
        max_output_tokens: Some(96),
    };

    let started_at = Instant::now();
    match llm.generate(request).await {
        Ok(response) => {
            let latency_ms = started_at.elapsed().as_millis();
            tracing::info!(
                agent_name,
                tick_id,
                model = %response.model,
                latency_ms,
                "llm summary generated for agent tick"
            );
            (
                trim_text(&response.text, LLM_SUMMARY_MAX_CHARS),
                true,
                Some(response.model),
                None,
                Some(latency_ms),
            )
        }
        Err(error) => {
            let latency_ms = started_at.elapsed().as_millis();
            tracing::warn!(
                agent_name,
                tick_id,
                latency_ms,
                error = %error,
                "llm failed during agent tick, using deterministic fallback"
            );
            (
                fallback,
                false,
                None,
                Some(error.to_string()),
                Some(latency_ms),
            )
        }
    }
}

fn fallback_summary(agent_name: &str, mood_label: &str) -> String {
    format!(
        "{agent_name} stays {mood_label} and continues a routine social action in the environment."
    )
}

fn trim_text(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.trim().to_owned();
    }
    input
        .chars()
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use anyhow::Result;
    use async_trait::async_trait;
    use chrono::Utc;
    use serde_json::Value;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    use crate::agent_core::{
        AgentCoreRepository, AgentEventRecord, AgentRecord, AgentStateRecord, NewAgentEvent,
        TickLeaseAcquireResult,
    };
    use crate::llm::{LlmGenerateRequest, LlmGenerateResponse, LlmPort};

    use super::{AgentTickExecutionStatus, AgentTickOrchestrator, AgentTickOrchestratorOutcome};

    #[derive(Default)]
    struct InMemoryAgentCoreRepository {
        agents: Mutex<HashMap<Uuid, AgentRecord>>,
        states: Mutex<HashMap<Uuid, AgentStateRecord>>,
        events: Mutex<Vec<AgentEventRecord>>,
        completed_ticks: Mutex<HashMap<Uuid, std::collections::HashSet<String>>>,
        active_leases: Mutex<HashMap<Uuid, String>>,
    }

    enum StubLlmMode {
        Success(String),
        Fail(String),
    }

    struct StubLlm {
        mode: StubLlmMode,
        calls: Arc<AtomicUsize>,
    }

    impl StubLlm {
        fn success(summary: &str, calls: Arc<AtomicUsize>) -> Self {
            Self {
                mode: StubLlmMode::Success(summary.to_owned()),
                calls,
            }
        }

        fn fail(message: &str, calls: Arc<AtomicUsize>) -> Self {
            Self {
                mode: StubLlmMode::Fail(message.to_owned()),
                calls,
            }
        }
    }

    #[async_trait]
    impl LlmPort for StubLlm {
        async fn generate(&self, _request: LlmGenerateRequest) -> Result<LlmGenerateResponse> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match &self.mode {
                StubLlmMode::Success(summary) => Ok(LlmGenerateResponse {
                    text: summary.clone(),
                    model: "stub-model".to_owned(),
                }),
                StubLlmMode::Fail(message) => Err(anyhow::anyhow!(message.clone())),
            }
        }
    }

    #[async_trait]
    impl AgentCoreRepository for InMemoryAgentCoreRepository {
        async fn get_agent(&self, agent_id: Uuid) -> Result<Option<AgentRecord>> {
            let agents = self.agents.lock().await;
            Ok(agents.get(&agent_id).cloned())
        }

        async fn get_agent_state(&self, agent_id: Uuid) -> Result<Option<AgentStateRecord>> {
            let states = self.states.lock().await;
            Ok(states.get(&agent_id).cloned())
        }

        async fn upsert_agent_state(&self, state: &AgentStateRecord) -> Result<()> {
            let mut states = self.states.lock().await;
            states.insert(state.agent_id, state.clone());
            Ok(())
        }

        async fn append_agent_event(&self, event: &NewAgentEvent) -> Result<AgentEventRecord> {
            let mut events = self.events.lock().await;
            let id = (events.len() as i64) + 1;
            let record = AgentEventRecord {
                id,
                agent_id: event.agent_id,
                event_type: event.event_type.clone(),
                description: event.description.clone(),
                payload_json: event.payload_json.clone(),
                occurred_at: Utc::now(),
            };
            events.push(record.clone());
            Ok(record)
        }

        async fn list_agent_events(
            &self,
            agent_id: Option<Uuid>,
            limit: u32,
        ) -> Result<Vec<AgentEventRecord>> {
            let events = self.events.lock().await;
            let mut filtered: Vec<AgentEventRecord> = events
                .iter()
                .filter(|record| match agent_id {
                    Some(id) => record.agent_id == Some(id),
                    None => true,
                })
                .cloned()
                .collect();
            filtered.reverse();
            filtered.truncate(limit as usize);
            Ok(filtered)
        }

        async fn has_completed_tick(&self, agent_id: Uuid, tick_id: &str) -> Result<bool> {
            let completed = self.completed_ticks.lock().await;
            Ok(completed
                .get(&agent_id)
                .map(|ticks| ticks.contains(tick_id))
                .unwrap_or(false))
        }

        async fn try_acquire_tick_lease(
            &self,
            agent_id: Uuid,
            tick_id: &str,
            _lease_ttl: Duration,
        ) -> Result<TickLeaseAcquireResult> {
            let mut leases = self.active_leases.lock().await;
            if leases.contains_key(&agent_id) {
                return Ok(TickLeaseAcquireResult::Busy);
            }
            leases.insert(agent_id, tick_id.to_owned());
            Ok(TickLeaseAcquireResult::Acquired)
        }

        async fn release_tick_lease(&self, agent_id: Uuid, tick_id: &str) -> Result<()> {
            let mut leases = self.active_leases.lock().await;
            if leases.get(&agent_id).map(|value| value.as_str()) == Some(tick_id) {
                leases.remove(&agent_id);
            }
            Ok(())
        }

        async fn record_completed_tick(&self, agent_id: Uuid, tick_id: &str) -> Result<()> {
            let mut completed = self.completed_ticks.lock().await;
            completed
                .entry(agent_id)
                .or_insert_with(HashSet::new)
                .insert(tick_id.to_owned());
            Ok(())
        }
    }

    #[tokio::test]
    async fn applies_tick_for_existing_agent() {
        let repository = Arc::new(InMemoryAgentCoreRepository::default());
        let agent_id = Uuid::new_v4();
        seed_agent(repository.as_ref(), agent_id, "Alice").await;

        let orchestrator = AgentTickOrchestrator::new(repository.clone());
        let outcome = orchestrator
            .run_agent_tick(agent_id, Some("tick-a".to_owned()))
            .await
            .expect("tick should execute");

        let AgentTickOrchestratorOutcome::Executed(result) = outcome else {
            panic!("expected executed outcome");
        };

        assert_eq!(result.status, AgentTickExecutionStatus::Applied);
        assert_eq!(result.agent_id, agent_id);
        assert_eq!(result.tick_id, "tick-a");
        assert_eq!(result.mood_label, "neutral");
        assert!(result.event_id.is_some());
    }

    #[tokio::test]
    async fn marks_missing_agent_without_writing_event() {
        let repository = Arc::new(InMemoryAgentCoreRepository::default());
        let orchestrator = AgentTickOrchestrator::new(repository.clone());
        let missing_agent_id = Uuid::new_v4();

        let outcome = orchestrator
            .run_agent_tick(missing_agent_id, Some("tick-missing".to_owned()))
            .await
            .expect("tick should finish");

        let AgentTickOrchestratorOutcome::Executed(result) = outcome else {
            panic!("expected executed outcome");
        };

        assert_eq!(result.status, AgentTickExecutionStatus::AgentMissing);
        assert_eq!(result.event_id, None);
    }

    #[tokio::test]
    async fn skips_duplicate_tick_id() {
        let repository = Arc::new(InMemoryAgentCoreRepository::default());
        let agent_id = Uuid::new_v4();
        seed_agent(repository.as_ref(), agent_id, "Bob").await;
        let orchestrator = AgentTickOrchestrator::new(repository.clone());

        let first = orchestrator
            .run_agent_tick(agent_id, Some("tick-dupe".to_owned()))
            .await
            .expect("first tick should execute");
        let second = orchestrator
            .run_agent_tick(agent_id, Some("tick-dupe".to_owned()))
            .await
            .expect("second tick should execute");

        assert!(matches!(first, AgentTickOrchestratorOutcome::Executed(_)));
        assert!(matches!(
            second,
            AgentTickOrchestratorOutcome::SkippedDuplicate
        ));
    }

    #[tokio::test]
    async fn skips_duplicate_from_repository_idempotency_history() {
        let repository = Arc::new(InMemoryAgentCoreRepository::default());
        let agent_id = Uuid::new_v4();
        seed_agent(repository.as_ref(), agent_id, "Nora").await;
        repository
            .record_completed_tick(agent_id, "tick-db-dupe")
            .await
            .expect("should seed completed tick");

        let orchestrator = AgentTickOrchestrator::new(repository);
        let outcome = orchestrator
            .run_agent_tick(agent_id, Some("tick-db-dupe".to_owned()))
            .await
            .expect("tick should complete");

        assert!(matches!(
            outcome,
            AgentTickOrchestratorOutcome::SkippedDuplicate
        ));
    }

    #[tokio::test]
    async fn skips_busy_when_global_lease_is_owned_by_other_worker() {
        let repository = Arc::new(InMemoryAgentCoreRepository::default());
        let agent_id = Uuid::new_v4();
        seed_agent(repository.as_ref(), agent_id, "Ivy").await;

        {
            let mut leases = repository.active_leases.lock().await;
            leases.insert(agent_id, "someone-else".to_owned());
        }

        let orchestrator = AgentTickOrchestrator::new(repository);
        let outcome = orchestrator
            .run_agent_tick(agent_id, Some("tick-busy".to_owned()))
            .await
            .expect("tick should complete");

        assert!(matches!(outcome, AgentTickOrchestratorOutcome::SkippedBusy));
    }

    #[tokio::test]
    async fn uses_llm_summary_when_available() {
        let repository = Arc::new(InMemoryAgentCoreRepository::default());
        let agent_id = Uuid::new_v4();
        seed_agent(repository.as_ref(), agent_id, "Lena").await;
        let calls = Arc::new(AtomicUsize::new(0));
        let llm: Arc<dyn LlmPort> = Arc::new(StubLlm::success(
            "Plans a cooperative conversation.",
            Arc::clone(&calls),
        ));

        let orchestrator =
            AgentTickOrchestrator::new(repository.clone()).with_optional_llm(Some(llm));
        let outcome = orchestrator
            .run_agent_tick(agent_id, Some("tick-llm-ok".to_owned()))
            .await
            .expect("tick should execute");

        assert!(matches!(outcome, AgentTickOrchestratorOutcome::Executed(_)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let events = repository.events.lock().await;
        let event = events.last().expect("event should exist");
        assert_eq!(event.payload_json["llm"]["used"], true);
        assert_eq!(event.payload_json["llm"]["model"], "stub-model");
        assert_eq!(
            event.payload_json["action_summary"],
            "Plans a cooperative conversation."
        );
    }

    #[tokio::test]
    async fn falls_back_when_llm_fails() {
        let repository = Arc::new(InMemoryAgentCoreRepository::default());
        let agent_id = Uuid::new_v4();
        seed_agent(repository.as_ref(), agent_id, "Mia").await;
        let calls = Arc::new(AtomicUsize::new(0));
        let llm: Arc<dyn LlmPort> =
            Arc::new(StubLlm::fail("simulated llm outage", Arc::clone(&calls)));

        let orchestrator =
            AgentTickOrchestrator::new(repository.clone()).with_optional_llm(Some(llm));
        let outcome = orchestrator
            .run_agent_tick(agent_id, Some("tick-llm-fail".to_owned()))
            .await
            .expect("tick should execute");

        assert!(matches!(outcome, AgentTickOrchestratorOutcome::Executed(_)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let events = repository.events.lock().await;
        let event = events.last().expect("event should exist");
        assert_eq!(event.payload_json["llm"]["used"], false);
        assert_eq!(event.payload_json["llm"]["configured"], true);
        assert!(
            event.payload_json["llm"]["error"]
                .as_str()
                .unwrap_or_default()
                .contains("simulated llm outage")
        );
        assert!(
            event.payload_json["action_summary"]
                .as_str()
                .unwrap_or_default()
                .contains("Mia")
        );
    }

    async fn seed_agent(repository: &InMemoryAgentCoreRepository, agent_id: Uuid, name: &str) {
        let mut agents = repository.agents.lock().await;
        agents.insert(
            agent_id,
            AgentRecord {
                id: agent_id,
                name: name.to_owned(),
                avatar_url: None,
                personality_json: Value::Object(Default::default()),
                created_at: Utc::now(),
            },
        );
    }
}
