use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

use crate::agent_core::{
    AgentCoreRepository, AgentStateRecord, AgentTickRunner, NewAgentEvent, TickRunOutcome,
};

const DEFAULT_TICK_HISTORY_PER_AGENT: usize = 256;

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
}

impl AgentTickOrchestrator {
    pub fn new(repository: Arc<dyn AgentCoreRepository>) -> Self {
        Self {
            repository,
            tick_runner: AgentTickRunner::new(DEFAULT_TICK_HISTORY_PER_AGENT),
        }
    }

    pub fn with_tick_runner(
        repository: Arc<dyn AgentCoreRepository>,
        tick_runner: AgentTickRunner,
    ) -> Self {
        Self {
            repository,
            tick_runner,
        }
    }

    pub async fn run_agent_tick(
        &self,
        agent_id: Uuid,
        tick_id: Option<String>,
    ) -> Result<AgentTickOrchestratorOutcome> {
        let tick_id = tick_id.unwrap_or_else(|| Uuid::new_v4().to_string());
        let repository = Arc::clone(&self.repository);
        let tick_id_for_closure = tick_id.clone();

        let tick_outcome = self
            .tick_runner
            .run_tick(&agent_id.to_string(), &tick_id, move || async move {
                execute_tick(repository.as_ref(), agent_id, &tick_id_for_closure).await
            })
            .await;

        match tick_outcome {
            TickRunOutcome::Executed(result) => {
                Ok(AgentTickOrchestratorOutcome::Executed(result?))
            }
            TickRunOutcome::SkippedBusy => Ok(AgentTickOrchestratorOutcome::SkippedBusy),
            TickRunOutcome::SkippedDuplicate => Ok(AgentTickOrchestratorOutcome::SkippedDuplicate),
        }
    }
}

async fn execute_tick(
    repository: &dyn AgentCoreRepository,
    agent_id: Uuid,
    tick_id: &str,
) -> Result<AgentTickExecutionResult> {
    let Some(agent) = repository.get_agent(agent_id).await? else {
        return Ok(AgentTickExecutionResult {
            agent_id,
            tick_id: tick_id.to_owned(),
            status: AgentTickExecutionStatus::AgentMissing,
            event_id: None,
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

    let event = repository
        .append_agent_event(&NewAgentEvent {
            agent_id: Some(agent_id),
            event_type: "agent.tick.executed".to_owned(),
            description: format!("Agent `{}` executed tick `{}`", agent.name, tick_id),
            payload_json: json!({
                "agent_id": agent_id,
                "tick_id": tick_id,
                "mood_label": next_mood,
                "valence": next_valence,
                "arousal": next_arousal
            }),
        })
        .await?;

    Ok(AgentTickExecutionResult {
        agent_id,
        tick_id: tick_id.to_owned(),
        status: AgentTickExecutionStatus::Applied,
        event_id: Some(event.id),
        mood_label: next_mood,
        valence: next_valence,
        arousal: next_arousal,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use anyhow::Result;
    use async_trait::async_trait;
    use chrono::Utc;
    use serde_json::Value;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    use crate::agent_core::{
        AgentCoreRepository, AgentEventRecord, AgentRecord, AgentStateRecord, NewAgentEvent,
    };

    use super::{
        AgentTickExecutionStatus, AgentTickOrchestrator, AgentTickOrchestratorOutcome,
    };

    #[derive(Default)]
    struct InMemoryAgentCoreRepository {
        agents: Mutex<HashMap<Uuid, AgentRecord>>,
        states: Mutex<HashMap<Uuid, AgentStateRecord>>,
        events: Mutex<Vec<AgentEventRecord>>,
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

    async fn seed_agent(
        repository: &InMemoryAgentCoreRepository,
        agent_id: Uuid,
        name: &str,
    ) {
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
