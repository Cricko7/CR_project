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
const PIPELINE_STAGE_MAX_CHARS: usize = 320;
const EVENT_DESCRIPTION_MAX_CHARS: usize = 180;
const EMOTION_INERTIA_VALENCE: f32 = 0.72;
const EMOTION_INERTIA_AROUSAL: f32 = 0.68;
const EMOTION_MAX_PERSONALITY_BIAS: f32 = 0.08;

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

#[derive(Debug, Clone)]
struct DecisionStageOutput {
    text: String,
    used_llm: bool,
    model: Option<String>,
    error: Option<String>,
    latency_ms: Option<u128>,
}

impl DecisionStageOutput {
    fn to_llm_json(&self) -> Value {
        json!({
            "used": self.used_llm,
            "model": self.model,
            "error": self.error,
            "latency_ms": self.latency_ms,
        })
    }
}

#[derive(Debug, Clone)]
struct AgentDecisionPipeline {
    reflection: DecisionStageOutput,
    goal: DecisionStageOutput,
    action_plan: DecisionStageOutput,
    execution: DecisionStageOutput,
}

impl AgentDecisionPipeline {
    fn action_summary(&self) -> String {
        self.execution.text.clone()
    }

    fn used_llm(&self) -> bool {
        self.reflection.used_llm
            || self.goal.used_llm
            || self.action_plan.used_llm
            || self.execution.used_llm
    }

    fn first_model(&self) -> Option<String> {
        for stage in [
            &self.reflection,
            &self.goal,
            &self.action_plan,
            &self.execution,
        ] {
            if let Some(model) = &stage.model {
                return Some(model.clone());
            }
        }
        None
    }

    fn first_error(&self) -> Option<String> {
        for stage in [
            &self.reflection,
            &self.goal,
            &self.action_plan,
            &self.execution,
        ] {
            if let Some(error) = &stage.error {
                return Some(error.clone());
            }
        }
        None
    }

    fn total_latency_ms(&self) -> Option<u128> {
        let mut total = 0u128;
        let mut has_latency = false;
        for stage in [
            &self.reflection,
            &self.goal,
            &self.action_plan,
            &self.execution,
        ] {
            if let Some(value) = stage.latency_ms {
                total = total.saturating_add(value);
                has_latency = true;
            }
        }
        if has_latency { Some(total) } else { None }
    }

    fn decision_json(&self) -> Value {
        json!({
            "reflection": self.reflection.text,
            "goal": self.goal.text,
            "action_plan": self.action_plan.text,
            "execution": self.execution.text,
        })
    }

    fn llm_json(&self, configured: bool) -> Value {
        json!({
            "configured": configured,
            "used": self.used_llm(),
            "model": self.first_model(),
            "error": self.first_error(),
            "latency_ms": self.total_latency_ms(),
            "stages": {
                "reflection": self.reflection.to_llm_json(),
                "goal": self.goal.to_llm_json(),
                "action_plan": self.action_plan.to_llm_json(),
                "execution": self.execution.to_llm_json(),
            }
        })
    }
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

    let (previous_valence, previous_arousal, previous_mood) = match current_state {
        Some(state) => (
            state.valence.clamp(-1.0, 1.0),
            state.arousal.clamp(-1.0, 1.0),
            state.mood_label,
        ),
        None => (0.0, 0.0, "neutral".to_owned()),
    };

    let decision_pipeline = run_decision_pipeline(
        llm.as_deref(),
        &agent.name,
        &agent.personality_json,
        tick_id,
        &previous_mood,
        previous_valence,
        previous_arousal,
    )
    .await;
    let action_summary = decision_pipeline.action_summary();

    let (next_valence, next_arousal, next_mood) = evolve_emotional_state(
        previous_valence,
        previous_arousal,
        &action_summary,
        &agent.personality_json,
    );

    repository
        .upsert_agent_state(&AgentStateRecord {
            agent_id,
            valence: next_valence,
            arousal: next_arousal,
            mood_label: next_mood.clone(),
            updated_at: now,
        })
        .await?;

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
                "decision_pipeline": decision_pipeline.decision_json(),
                "emotion": {
                    "previous": {
                        "mood_label": previous_mood,
                        "valence": previous_valence,
                        "arousal": previous_arousal,
                    },
                    "next": {
                        "mood_label": next_mood,
                        "valence": next_valence,
                        "arousal": next_arousal,
                    },
                    "delta": {
                        "valence": next_valence - previous_valence,
                        "arousal": next_arousal - previous_arousal,
                    }
                },
                "action_summary": action_summary,
                "llm": decision_pipeline.llm_json(llm.is_some())
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

async fn run_decision_pipeline(
    llm: Option<&dyn LlmPort>,
    agent_name: &str,
    personality_json: &Value,
    tick_id: &str,
    mood_label: &str,
    valence: f32,
    arousal: f32,
) -> AgentDecisionPipeline {
    let reflection_fallback =
        format!("{agent_name} reflects on the current world state and their {mood_label} mood.");
    let reflection = generate_pipeline_stage(
        llm,
        "reflection",
        Some(format!(
            "You are the reflection stage of autonomous agent `{agent_name}`. Personality JSON: {personality_json}. Return one concise sentence."
        )),
        format!(
            "Tick {tick_id}. Mood: {mood_label}. Valence: {valence:.2}. Arousal: {arousal:.2}. Write a brief self-reflection."
        ),
        reflection_fallback,
        PIPELINE_STAGE_MAX_CHARS,
    )
    .await;

    let goal_fallback =
        format!("{agent_name} sets a short-term goal to improve social stability and progress.");
    let goal = generate_pipeline_stage(
        llm,
        "goal_selection",
        Some(format!(
            "You are the goal-selection stage for agent `{agent_name}`. Return one concrete goal sentence."
        )),
        format!(
            "Tick {tick_id}. Reflection: {}. Mood: {mood_label}. Choose one immediate goal.",
            reflection.text
        ),
        goal_fallback,
        PIPELINE_STAGE_MAX_CHARS,
    )
    .await;

    let action_plan_fallback = format!(
        "{agent_name} plans a concrete action: contact a nearby agent, exchange context, and adapt."
    );
    let action_plan = generate_pipeline_stage(
        llm,
        "action_planning",
        Some(format!(
            "You are the action-planning stage for agent `{agent_name}`. Return one concrete immediate plan."
        )),
        format!(
            "Tick {tick_id}. Reflection: {}. Goal: {}. Produce an immediate action plan.",
            reflection.text, goal.text
        ),
        action_plan_fallback,
        PIPELINE_STAGE_MAX_CHARS,
    )
    .await;

    let execution_fallback = fallback_execution_summary(agent_name, mood_label, &action_plan.text);
    let execution = generate_pipeline_stage(
        llm,
        "execution",
        Some(format!(
            "You are the execution stage for agent `{agent_name}`. Simulate execution result in 1-2 short sentences."
        )),
        format!(
            "Tick {tick_id}. Goal: {}. Plan: {}. Mood: {mood_label}. Return execution result and immediate side-effect.",
            goal.text, action_plan.text
        ),
        execution_fallback,
        LLM_SUMMARY_MAX_CHARS,
    )
    .await;

    AgentDecisionPipeline {
        reflection,
        goal,
        action_plan,
        execution,
    }
}

async fn generate_pipeline_stage(
    llm: Option<&dyn LlmPort>,
    stage_name: &'static str,
    system_prompt: Option<String>,
    user_prompt: String,
    fallback: String,
    max_chars: usize,
) -> DecisionStageOutput {
    let Some(llm) = llm else {
        return DecisionStageOutput {
            text: fallback,
            used_llm: false,
            model: None,
            error: Some("llm_not_configured".to_owned()),
            latency_ms: None,
        };
    };

    let request = LlmGenerateRequest {
        system_prompt,
        user_prompt,
        temperature: Some(0.4),
        max_output_tokens: Some(120),
    };

    let started_at = Instant::now();
    match llm.generate(request).await {
        Ok(response) => DecisionStageOutput {
            text: trim_text(&response.text, max_chars),
            used_llm: true,
            model: Some(response.model),
            error: None,
            latency_ms: Some(started_at.elapsed().as_millis()),
        },
        Err(error) => {
            let latency_ms = started_at.elapsed().as_millis();
            tracing::warn!(
                stage = stage_name,
                latency_ms,
                error = %error,
                "llm stage failed, using deterministic fallback"
            );
            DecisionStageOutput {
                text: fallback,
                used_llm: false,
                model: None,
                error: Some(error.to_string()),
                latency_ms: Some(latency_ms),
            }
        }
    }
}

fn fallback_execution_summary(agent_name: &str, mood_label: &str, action_plan: &str) -> String {
    format!(
        "{agent_name} ({mood_label}) executes plan: {}",
        trim_text(action_plan, EVENT_DESCRIPTION_MAX_CHARS)
    )
}

fn evolve_emotional_state(
    previous_valence: f32,
    previous_arousal: f32,
    action_summary: &str,
    personality_json: &Value,
) -> (f32, f32, String) {
    let (summary_valence_delta, summary_arousal_delta) = summary_emotion_delta(action_summary);
    let (personality_valence_bias, personality_arousal_bias) =
        personality_emotion_bias(personality_json);

    let next_valence = (previous_valence * EMOTION_INERTIA_VALENCE
        + summary_valence_delta
        + personality_valence_bias)
        .clamp(-1.0, 1.0);
    let next_arousal = (previous_arousal * EMOTION_INERTIA_AROUSAL
        + summary_arousal_delta
        + personality_arousal_bias)
        .clamp(-1.0, 1.0);

    let next_mood = classify_mood(next_valence, next_arousal);
    (next_valence, next_arousal, next_mood)
}

fn summary_emotion_delta(summary: &str) -> (f32, f32) {
    let normalized = summary.to_lowercase();
    let positive_hits = keyword_hits(
        &normalized,
        &[
            "cooperate",
            "cooperative",
            "support",
            "friend",
            "help",
            "discover",
            "succeed",
            "trust",
            "calm",
            "joy",
            "progress",
            "resolve",
        ],
    );
    let negative_hits = keyword_hits(
        &normalized,
        &[
            "conflict", "argue", "fail", "threat", "danger", "panic", "fear", "angry", "sad",
            "stress", "attack", "loss",
        ],
    );
    let high_arousal_hits = keyword_hits(
        &normalized,
        &[
            "urgent", "quickly", "danger", "conflict", "panic", "excited", "intense", "rush",
            "alert",
        ],
    );
    let low_arousal_hits = keyword_hits(
        &normalized,
        &[
            "calm", "steady", "routine", "rest", "observe", "reflect", "slow", "quiet",
        ],
    );

    let valence_delta = ((positive_hits as f32 - negative_hits as f32) * 0.07).clamp(-0.25, 0.25);
    let arousal_delta =
        ((high_arousal_hits as f32 - low_arousal_hits as f32) * 0.06).clamp(-0.24, 0.24);
    (valence_delta, arousal_delta)
}

fn personality_emotion_bias(personality_json: &Value) -> (f32, f32) {
    let mut valence_bias: f32 = 0.0;
    let mut arousal_bias: f32 = 0.0;
    for trait_name in extract_personality_traits(personality_json) {
        match trait_name.as_str() {
            "optimistic" | "friendly" | "empathetic" | "cooperative" => valence_bias += 0.03,
            "curious" | "adventurous" => {
                valence_bias += 0.02;
                arousal_bias += 0.02;
            }
            "anxious" | "neurotic" => {
                valence_bias -= 0.03;
                arousal_bias += 0.04;
            }
            "calm" | "patient" => {
                valence_bias += 0.01;
                arousal_bias -= 0.03;
            }
            "aggressive" => {
                valence_bias -= 0.02;
                arousal_bias += 0.05;
            }
            "sarcastic" => valence_bias -= 0.01,
            _ => {}
        }
    }

    (
        valence_bias.clamp(-EMOTION_MAX_PERSONALITY_BIAS, EMOTION_MAX_PERSONALITY_BIAS),
        arousal_bias.clamp(-EMOTION_MAX_PERSONALITY_BIAS, EMOTION_MAX_PERSONALITY_BIAS),
    )
}

fn extract_personality_traits(personality_json: &Value) -> Vec<String> {
    let Some(traits) = personality_json.get("traits").and_then(Value::as_array) else {
        return Vec::new();
    };

    traits
        .iter()
        .filter_map(Value::as_str)
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

fn keyword_hits(input: &str, keywords: &[&str]) -> usize {
    keywords.iter().filter(|word| input.contains(*word)).count()
}

pub fn classify_mood(valence: f32, arousal: f32) -> String {
    if valence >= 0.45 && arousal >= 0.35 {
        return "excited".to_owned();
    }
    if valence >= 0.45 && arousal <= -0.2 {
        return "content".to_owned();
    }
    if valence >= 0.2 && arousal <= 0.3 {
        return "calm".to_owned();
    }
    if valence <= -0.45 && arousal >= 0.35 {
        return "angry".to_owned();
    }
    if valence <= -0.35 && arousal <= -0.15 {
        return "sad".to_owned();
    }
    if arousal >= 0.6 && valence.abs() < 0.2 {
        return "anxious".to_owned();
    }
    if arousal <= -0.45 && valence.abs() < 0.2 {
        return "tired".to_owned();
    }
    "neutral".to_owned()
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
        AgentCoreRepository, AgentEventRecord, AgentRecord, AgentStateRecord, InterventionRecord,
        MessageRecord, NewAgentEvent, NewIntervention, NewMessage, RelationshipRecord,
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

        async fn list_agent_events_after_id(
            &self,
            agent_id: Option<Uuid>,
            after_id: i64,
            limit: u32,
        ) -> Result<Vec<AgentEventRecord>> {
            let events = self.events.lock().await;
            let mut filtered: Vec<AgentEventRecord> = events
                .iter()
                .filter(|record| match agent_id {
                    Some(id) => record.agent_id == Some(id),
                    None => true,
                })
                .filter(|record| record.id > after_id)
                .cloned()
                .collect();
            filtered.sort_by_key(|record| record.id);
            filtered.truncate(limit as usize);
            Ok(filtered)
        }

        async fn latest_event_id(&self, agent_id: Option<Uuid>) -> Result<Option<i64>> {
            let events = self.events.lock().await;
            Ok(events
                .iter()
                .filter(|record| match agent_id {
                    Some(id) => record.agent_id == Some(id),
                    None => true,
                })
                .map(|record| record.id)
                .max())
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

        async fn enqueue_message(&self, new_message: &NewMessage) -> Result<MessageRecord> {
            Ok(MessageRecord {
                id: 1,
                sender_type: new_message.sender_type.clone(),
                sender_id: new_message.sender_id,
                receiver_agent_id: new_message.receiver_agent_id,
                content: new_message.content.clone(),
                status: "queued".to_owned(),
                created_at: Utc::now(),
            })
        }

        async fn claim_queued_messages(
            &self,
            _limit: u32,
            _claim_timeout: Duration,
        ) -> Result<Vec<MessageRecord>> {
            Ok(Vec::new())
        }

        async fn mark_message_delivered(&self, _message_id: i64) -> Result<()> {
            Ok(())
        }

        async fn mark_message_failed(&self, _message_id: i64, _error: &str) -> Result<()> {
            Ok(())
        }

        async fn list_agent_messages(
            &self,
            _receiver_agent_id: Uuid,
            _limit: u32,
        ) -> Result<Vec<MessageRecord>> {
            Ok(Vec::new())
        }

        async fn upsert_relationship_interaction(
            &self,
            left_agent_id: Uuid,
            right_agent_id: Uuid,
            affinity_delta: f32,
            interaction_summary: &str,
            interaction_at: chrono::DateTime<Utc>,
        ) -> Result<RelationshipRecord> {
            Ok(RelationshipRecord {
                id: 1,
                agent_a: left_agent_id,
                agent_b: right_agent_id,
                affinity_score: affinity_delta.clamp(-1.0, 1.0),
                history_summary: interaction_summary.to_owned(),
                last_interaction_at: Some(interaction_at),
                created_at: Utc::now(),
            })
        }

        async fn list_agent_relationships(
            &self,
            _agent_id: Uuid,
            _limit: u32,
        ) -> Result<Vec<RelationshipRecord>> {
            Ok(Vec::new())
        }

        async fn list_relationships(&self, _limit: u32) -> Result<Vec<RelationshipRecord>> {
            Ok(Vec::new())
        }

        async fn append_intervention(
            &self,
            intervention: &NewIntervention,
        ) -> Result<InterventionRecord> {
            Ok(InterventionRecord {
                id: 1,
                admin_user_id: intervention.admin_user_id.clone(),
                action_type: intervention.action_type.clone(),
                payload_json: intervention.payload_json.clone(),
                result_status: intervention.result_status.clone(),
                created_at: Utc::now(),
            })
        }

        async fn list_interventions(&self, _limit: u32) -> Result<Vec<InterventionRecord>> {
            Ok(Vec::new())
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
        assert_eq!(calls.load(Ordering::SeqCst), 4);

        let events = repository.events.lock().await;
        let event = events.last().expect("event should exist");
        assert_eq!(event.payload_json["llm"]["used"], true);
        assert_eq!(event.payload_json["llm"]["model"], "stub-model");
        assert!(
            event.payload_json["decision_pipeline"]["reflection"]
                .as_str()
                .unwrap_or_default()
                .contains("Plans a cooperative conversation")
        );
        assert_eq!(
            event.payload_json["llm"]["stages"]["execution"]["used"],
            true
        );
        assert_eq!(
            event.payload_json["action_summary"],
            "Plans a cooperative conversation."
        );
        assert!(event.payload_json["valence"].as_f64().unwrap_or_default() > 0.0);
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
        assert_eq!(calls.load(Ordering::SeqCst), 4);

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

    #[tokio::test]
    async fn updates_emotion_state_from_action_summary() {
        let repository = Arc::new(InMemoryAgentCoreRepository::default());
        let agent_id = Uuid::new_v4();
        seed_agent(repository.as_ref(), agent_id, "Em").await;
        let calls = Arc::new(AtomicUsize::new(0));
        let llm: Arc<dyn LlmPort> = Arc::new(StubLlm::success(
            "Faced urgent conflict and danger, reacts quickly.",
            Arc::clone(&calls),
        ));

        let orchestrator =
            AgentTickOrchestrator::new(repository.clone()).with_optional_llm(Some(llm));
        let outcome = orchestrator
            .run_agent_tick(agent_id, Some("tick-emotion-shift".to_owned()))
            .await
            .expect("tick should execute");

        let AgentTickOrchestratorOutcome::Executed(result) = outcome else {
            panic!("expected executed outcome");
        };

        assert!(result.arousal > 0.0);
        assert!(result.valence < 0.0);
        assert!(["angry", "anxious", "neutral"].contains(&result.mood_label.as_str()));
    }

    #[tokio::test]
    async fn applies_personality_bias_to_emotion_dynamics() {
        let repository = Arc::new(InMemoryAgentCoreRepository::default());
        let agent_id = Uuid::new_v4();
        seed_agent_with_personality(
            repository.as_ref(),
            agent_id,
            "Bias",
            serde_json::json!({ "traits": ["anxious"] }),
        )
        .await;

        let orchestrator = AgentTickOrchestrator::new(repository);
        let outcome = orchestrator
            .run_agent_tick(agent_id, Some("tick-personality-bias".to_owned()))
            .await
            .expect("tick should execute");

        let AgentTickOrchestratorOutcome::Executed(result) = outcome else {
            panic!("expected executed outcome");
        };

        assert!(result.valence < 0.0);
    }

    #[test]
    fn classifies_core_mood_buckets() {
        assert_eq!(super::classify_mood(0.6, 0.5), "excited");
        assert_eq!(super::classify_mood(-0.6, 0.6), "angry");
        assert_eq!(super::classify_mood(-0.5, -0.3), "sad");
        assert_eq!(super::classify_mood(0.1, -0.6), "tired");
        assert_eq!(super::classify_mood(0.0, 0.0), "neutral");
    }

    async fn seed_agent(repository: &InMemoryAgentCoreRepository, agent_id: Uuid, name: &str) {
        seed_agent_with_personality(
            repository,
            agent_id,
            name,
            Value::Object(Default::default()),
        )
        .await;
    }

    async fn seed_agent_with_personality(
        repository: &InMemoryAgentCoreRepository,
        agent_id: Uuid,
        name: &str,
        personality_json: Value,
    ) {
        let mut agents = repository.agents.lock().await;
        agents.insert(
            agent_id,
            AgentRecord {
                id: agent_id,
                name: name.to_owned(),
                avatar_url: None,
                personality_json,
                created_at: Utc::now(),
            },
        );
    }
}
