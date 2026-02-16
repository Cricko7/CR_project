use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Utc;
use serde_json::json;
use sim_backend::agent_core::{
    AgentCoreRepository, AgentRecord, AgentTickExecutionStatus, AgentTickOrchestrator,
    AgentTickOrchestratorOutcome, DEFAULT_SIMULATION_TIME_SCALE, MAX_SIMULATION_TIME_SCALE,
    MIN_SIMULATION_TIME_SCALE, MessageRecord, NewAgentEvent, NewMessage,
};
use sim_backend::app::config::WorkerConfig;
use sim_backend::app::observability::init_tracing;
use sim_backend::app::runtime::ServiceRuntime;
use sim_backend::infrastructure::gemini::GeminiClient;
use sim_backend::infrastructure::gemini_embedding::GeminiEmbeddingClient;
use sim_backend::infrastructure::postgres::{
    PostgresAgentCoreRepository, PostgresMemoryRepository, ensure_ready,
};
use sim_backend::infrastructure::qdrant::QdrantVectorStore;
use sim_backend::llm::LlmPort;
use sim_backend::memory::{
    MemoryRepository, MemoryService, MemoryVectorStore, SimpleHashEmbedder, TextEmbedder,
};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use uuid::Uuid;

const MESSAGE_CLAIM_TIMEOUT: Duration = Duration::from_secs(60);
const MESSAGE_EVENT_DESCRIPTION_CHARS: usize = 200;
const MIN_SIMULATION_SLEEP: Duration = Duration::from_millis(100);
const CONVERSATION_MESSAGE_MAX_CHARS: usize = 280;
const CONVERSATION_TOPICS: &[&str] = &[
    "how to coordinate the next exploration step",
    "sharing quick status updates",
    "how to split responsibilities efficiently",
    "who can help with the next risky move",
    "a short plan for safer cooperation",
];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = WorkerConfig::from_env()?;
    init_tracing(&config.common.service_name, &config.common.log_level)?;
    let db_pool = ensure_ready(&config.database).await?;
    let gemini_client: Option<Arc<dyn LlmPort>> = match config.gemini.clone() {
        Some(gemini_config) => {
            tracing::info!(model = %gemini_config.model, "gemini client configured");
            Some(Arc::new(GeminiClient::new(gemini_config)?) as Arc<dyn LlmPort>)
        }
        None => {
            tracing::warn!("GEMINI_API_KEY is not set; worker runs without llm integration");
            None
        }
    };

    let vector_store: Arc<dyn MemoryVectorStore> =
        Arc::new(QdrantVectorStore::new(config.qdrant.clone())?);
    vector_store.ensure_collection().await?;
    let memory_repository: Arc<dyn MemoryRepository> =
        Arc::new(PostgresMemoryRepository::new(db_pool.clone()));
    let embedder: Arc<dyn TextEmbedder> = match config.gemini.clone() {
        Some(gemini_config) => Arc::new(GeminiEmbeddingClient::new(gemini_config)?),
        None => Arc::new(SimpleHashEmbedder::new(config.qdrant.vector_size as usize)),
    };
    let memory_service = Arc::new(MemoryService::new(
        memory_repository,
        vector_store,
        embedder,
        gemini_client.clone(),
        config.qdrant.vector_size as usize,
    ));

    let postgres_agent_repository = Arc::new(PostgresAgentCoreRepository::new(db_pool));
    let repository: Arc<dyn AgentCoreRepository> = postgres_agent_repository.clone();
    let orchestrator =
        AgentTickOrchestrator::new(repository.clone()).with_optional_llm(gemini_client);

    let mut runtime = ServiceRuntime::new(
        config.common.service_name.clone(),
        config.common.shutdown_timeout,
    );
    let cancellation = runtime.cancellation_token();

    let tick_interval = config.tick_interval;
    let agent_ids = config.agent_ids.clone();
    let tick_concurrency = config.tick_concurrency as usize;
    let tick_orchestrator = orchestrator.clone();
    let tick_memory_service = Arc::clone(&memory_service);
    let tick_scale_repository = repository.clone();
    let tick_token = cancellation.clone();
    runtime.spawn("agent_tick_worker", async move {
        if agent_ids.is_empty() {
            tracing::warn!(
                "WORKER_AGENT_IDS is empty; tick worker is idle until agent ids are configured"
            );
        }

        let semaphore = Arc::new(Semaphore::new(tick_concurrency));
        let mut first_tick = true;
        loop {
            let wait_duration = if first_tick {
                first_tick = false;
                Duration::ZERO
            } else {
                simulation_wait_duration(
                    tick_scale_repository.as_ref(),
                    tick_interval,
                    "agent_tick_worker",
                )
                .await
            };

            tokio::select! {
                _ = tick_token.cancelled() => {
                    tracing::info!("agent tick worker received shutdown");
                    break;
                }
                _ = tokio::time::sleep(wait_duration) => {
                    let mut in_flight = JoinSet::new();
                    for agent_id in agent_ids.iter().copied() {
                        let permit_pool = Arc::clone(&semaphore);
                        let orchestrator = tick_orchestrator.clone();
                        let memory_service = Arc::clone(&tick_memory_service);
                        in_flight.spawn(async move {
                            let _permit = permit_pool
                                .acquire_owned()
                                .await
                                .expect("tick concurrency semaphore should remain open");
                            process_agent_tick(orchestrator, memory_service, agent_id).await;
                        });
                    }

                    while let Some(joined) = in_flight.join_next().await {
                        if let Err(error) = joined {
                            tracing::error!(error = %error, "agent tick task panicked");
                        }
                    }
                }
            }
        }
        Ok(())
    });

    let mood_decay_interval = config.mood_decay_interval;
    let mood_decay_step = config.mood_decay_step;
    let mood_decay_repository = postgres_agent_repository.clone();
    let mood_token = cancellation.clone();
    runtime.spawn("mood_decay_worker", async move {
        let mut first_tick = true;
        loop {
            let wait_duration = if first_tick {
                first_tick = false;
                Duration::ZERO
            } else {
                simulation_wait_duration(
                    mood_decay_repository.as_ref(),
                    mood_decay_interval,
                    "mood_decay_worker",
                )
                .await
            };

            tokio::select! {
                _ = mood_token.cancelled() => {
                    tracing::info!("mood decay worker received shutdown");
                    break;
                }
                _ = tokio::time::sleep(wait_duration) => {
                    match mood_decay_repository.apply_global_mood_decay(mood_decay_step).await {
                        Ok(updated) => {
                            tracing::debug!(
                                updated_states = updated,
                                decay_step = mood_decay_step,
                                "mood decay tick applied"
                            );
                        }
                        Err(error) => {
                            tracing::error!(error = %error, "mood decay worker failed");
                        }
                    }
                }
            }
        }
        Ok(())
    });

    let embed_interval = config.memory.embed_interval;
    let embed_batch = config.memory.embed_batch_size;
    let embed_service = Arc::clone(&memory_service);
    let embed_token = cancellation.clone();
    runtime.spawn("memory_embedding_worker", async move {
        let mut interval = tokio::time::interval(embed_interval);
        loop {
            tokio::select! {
                _ = embed_token.cancelled() => {
                    tracing::info!("memory embedding worker received shutdown");
                    break;
                }
                _ = interval.tick() => {
                    match embed_service.process_pending_embeddings(embed_batch).await {
                        Ok(summary) => {
                            if summary.processed > 0 {
                                tracing::info!(
                                    processed = summary.processed,
                                    succeeded = summary.succeeded,
                                    failed = summary.failed,
                                    retried = summary.retried,
                                    dead_lettered = summary.dead_lettered,
                                    "memory embeddings processed"
                                );
                            }
                        }
                        Err(error) => {
                            tracing::error!(error = %error, "memory embedding worker failed");
                        }
                    }
                }
            }
        }
        Ok(())
    });

    let summary_interval = config.memory.summary_interval;
    let summary_max_active = config.memory.max_active_per_agent;
    let summary_batch_size = config.memory.summary_batch_size;
    let summary_agent_ids = config.agent_ids.clone();
    let summary_service = Arc::clone(&memory_service);
    let summary_token = cancellation.clone();
    runtime.spawn("memory_summarization_worker", async move {
        let mut interval = tokio::time::interval(summary_interval);
        loop {
            tokio::select! {
                _ = summary_token.cancelled() => {
                    tracing::info!("memory summarization worker received shutdown");
                    break;
                }
                _ = interval.tick() => {
                    for agent_id in &summary_agent_ids {
                        match summary_service
                            .summarize_overflow(*agent_id, summary_max_active, summary_batch_size)
                            .await
                        {
                            Ok(result) if result.created_summary => {
                                tracing::info!(
                                    agent_id = %agent_id,
                                    source_count = result.source_count,
                                    summary_id = ?result.summary_entry_id,
                                    "memory overflow summarized"
                                );
                            }
                            Ok(_) => {}
                            Err(error) => {
                                tracing::error!(agent_id = %agent_id, error = %error, "memory summarization failed");
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    });

    let message_interval = config.message_interval;
    let message_batch_size = config.message_batch_size;
    let message_repository = repository.clone();
    let message_token = cancellation.clone();
    runtime.spawn("message_delivery_worker", async move {
        let mut first_tick = true;
        loop {
            let wait_duration = if first_tick {
                first_tick = false;
                Duration::ZERO
            } else {
                simulation_wait_duration(
                    message_repository.as_ref(),
                    message_interval,
                    "message_delivery_worker",
                )
                .await
            };

            tokio::select! {
                _ = message_token.cancelled() => {
                    tracing::info!("message delivery worker received shutdown");
                    break;
                }
                _ = tokio::time::sleep(wait_duration) => {
                    match message_repository
                        .claim_queued_messages(message_batch_size, MESSAGE_CLAIM_TIMEOUT)
                        .await
                    {
                        Ok(messages) => {
                            for message in messages {
                                process_agent_message(&*message_repository, &message).await;
                            }
                        }
                        Err(error) => {
                            tracing::error!(error = %error, "message delivery worker failed to claim messages");
                        }
                    }
                }
            }
        }
        Ok(())
    });

    let conversation_repository = repository.clone();
    let conversation_token = cancellation.clone();
    let conversation_scan_interval = config.conversation_scan_interval;
    let conversation_min_interval = config.conversation_min_interval;
    let conversation_max_interval = config.conversation_max_interval;
    let conversation_agent_limit = config.conversation_agent_limit;
    runtime.spawn("conversation_seed_worker", async move {
        let mut known_agent_ids = HashSet::<Uuid>::new();
        let mut initialized = false;
        let mut random_due_at = tokio::time::Instant::now();

        loop {
            tokio::select! {
                _ = conversation_token.cancelled() => {
                    tracing::info!("conversation seed worker received shutdown");
                    break;
                }
                _ = tokio::time::sleep(conversation_scan_interval) => {
                    let agents = match conversation_repository.list_agents(conversation_agent_limit).await {
                        Ok(items) => items,
                        Err(error) => {
                            tracing::error!(error = %error, "conversation seed worker failed to list agents");
                            continue;
                        }
                    };

                    let current_ids: HashSet<Uuid> = agents.iter().map(|agent| agent.id).collect();
                    if !initialized {
                        known_agent_ids = current_ids;
                        initialized = true;
                        let next_delay = simulation_wait_duration(
                            conversation_repository.as_ref(),
                            random_duration_between(conversation_min_interval, conversation_max_interval),
                            "conversation_seed_worker",
                        )
                        .await;
                        random_due_at = tokio::time::Instant::now() + next_delay;
                        continue;
                    }

                    let new_agents: Vec<AgentRecord> = agents
                        .iter()
                        .filter(|agent| !known_agent_ids.contains(&agent.id))
                        .cloned()
                        .collect();
                    known_agent_ids = current_ids;

                    for new_agent in &new_agents {
                        match seed_onboarding_conversation(conversation_repository.as_ref(), new_agent, &agents).await {
                            Ok(enqueued) if enqueued > 0 => {
                                tracing::info!(
                                    agent_id = %new_agent.id,
                                    agent_name = %new_agent.name,
                                    enqueued_messages = enqueued,
                                    "seeded onboarding conversation for new agent"
                                );
                            }
                            Ok(_) => {}
                            Err(error) => {
                                tracing::error!(
                                    agent_id = %new_agent.id,
                                    error = %error,
                                    "failed to seed onboarding conversation for new agent"
                                );
                            }
                        }
                    }

                    if tokio::time::Instant::now() >= random_due_at {
                        match seed_random_conversation(conversation_repository.as_ref(), &agents).await {
                            Ok(enqueued) if enqueued > 0 => {
                                tracing::debug!(enqueued_messages = enqueued, "seeded random conversation");
                            }
                            Ok(_) => {}
                            Err(error) => {
                                tracing::error!(error = %error, "failed to seed random conversation");
                            }
                        }

                        let next_delay = simulation_wait_duration(
                            conversation_repository.as_ref(),
                            random_duration_between(conversation_min_interval, conversation_max_interval),
                            "conversation_seed_worker",
                        )
                        .await;
                        random_due_at = tokio::time::Instant::now() + next_delay;
                    }
                }
            }
        }
        Ok(())
    });

    runtime.run_until_shutdown().await
}

async fn simulation_wait_duration(
    repository: &dyn AgentCoreRepository,
    base_interval: Duration,
    worker_name: &'static str,
) -> Duration {
    let time_scale = match repository.get_time_scale().await {
        Ok(record) => sanitize_time_scale(record.time_scale, worker_name),
        Err(error) => {
            tracing::warn!(
                worker = worker_name,
                error = %error,
                fallback_time_scale = DEFAULT_SIMULATION_TIME_SCALE,
                "failed to read simulation time scale; using default"
            );
            DEFAULT_SIMULATION_TIME_SCALE
        }
    };

    let scaled = base_interval.as_secs_f64() / f64::from(time_scale);
    Duration::from_secs_f64(scaled.max(MIN_SIMULATION_SLEEP.as_secs_f64()))
}

fn sanitize_time_scale(raw: f32, worker_name: &'static str) -> f32 {
    if !raw.is_finite() {
        tracing::warn!(
            worker = worker_name,
            received_time_scale = %raw,
            fallback_time_scale = DEFAULT_SIMULATION_TIME_SCALE,
            "received non-finite simulation time scale; using default"
        );
        return DEFAULT_SIMULATION_TIME_SCALE;
    }

    let clamped = raw.clamp(MIN_SIMULATION_TIME_SCALE, MAX_SIMULATION_TIME_SCALE);
    if (clamped - raw).abs() > f32::EPSILON {
        tracing::warn!(
            worker = worker_name,
            received_time_scale = %raw,
            applied_time_scale = %clamped,
            "simulation time scale was out of range and has been clamped"
        );
    }
    clamped
}

async fn seed_onboarding_conversation(
    repository: &dyn AgentCoreRepository,
    new_agent: &AgentRecord,
    all_agents: &[AgentRecord],
) -> anyhow::Result<u32> {
    let peers: Vec<AgentRecord> = all_agents
        .iter()
        .filter(|agent| agent.id != new_agent.id)
        .cloned()
        .collect();
    if peers.is_empty() {
        return Ok(0);
    }

    let topic = pick_random_topic();
    if peers.len() == 1 {
        let peer = &peers[0];
        enqueue_agent_to_agent_message(
            repository,
            new_agent,
            peer,
            format!(
                "Hi {}, I'm {}. Can we cooperate on {}?",
                peer.name, new_agent.name, topic
            ),
        )
        .await?;
        return Ok(1);
    }

    let trio = choose_distinct_agents(&peers, 2);
    if trio.len() < 2 {
        return Ok(0);
    }
    let left = &trio[0];
    let right = &trio[1];

    enqueue_agent_to_agent_message(
        repository,
        new_agent,
        left,
        format!(
            "Hi {}, I'm {}. Let's cooperate on {}.",
            left.name, new_agent.name, topic
        ),
    )
    .await?;
    enqueue_agent_to_agent_message(
        repository,
        left,
        right,
        format!(
            "{} just joined us. Can we coordinate this together?",
            new_agent.name
        ),
    )
    .await?;
    enqueue_agent_to_agent_message(
        repository,
        right,
        new_agent,
        format!(
            "Welcome {}, I'm in. Let's support each other on {}.",
            new_agent.name, topic
        ),
    )
    .await?;
    Ok(3)
}

async fn seed_random_conversation(
    repository: &dyn AgentCoreRepository,
    all_agents: &[AgentRecord],
) -> anyhow::Result<u32> {
    if all_agents.len() < 2 {
        return Ok(0);
    }

    let group_size = if all_agents.len() >= 3 && random_bool(55) {
        3
    } else {
        2
    };
    let participants = choose_distinct_agents(all_agents, group_size);
    if participants.len() < 2 {
        return Ok(0);
    }

    let topic = pick_random_topic();
    if participants.len() == 2 {
        let first = &participants[0];
        let second = &participants[1];
        enqueue_agent_to_agent_message(
            repository,
            first,
            second,
            format!(
                "{}, quick sync: can we cooperate on {}?",
                second.name, topic
            ),
        )
        .await?;

        if random_bool(70) {
            enqueue_agent_to_agent_message(
                repository,
                second,
                first,
                format!(
                    "Yes {}, I can support that. Thanks for starting the chat.",
                    first.name
                ),
            )
            .await?;
            return Ok(2);
        }
        return Ok(1);
    }

    let a = &participants[0];
    let b = &participants[1];
    let c = &participants[2];

    enqueue_agent_to_agent_message(
        repository,
        a,
        b,
        format!(
            "{}, let's coordinate {} and keep everyone aligned.",
            b.name, topic
        ),
    )
    .await?;
    enqueue_agent_to_agent_message(
        repository,
        b,
        c,
        format!(
            "{}, joining you both on {}. I can help with support.",
            c.name, topic
        ),
    )
    .await?;
    enqueue_agent_to_agent_message(
        repository,
        c,
        a,
        format!(
            "{}, agreed. Let's cooperate and share updates every step.",
            a.name
        ),
    )
    .await?;
    Ok(3)
}

async fn enqueue_agent_to_agent_message(
    repository: &dyn AgentCoreRepository,
    sender: &AgentRecord,
    receiver: &AgentRecord,
    content: String,
) -> anyhow::Result<()> {
    if sender.id == receiver.id {
        return Ok(());
    }

    repository
        .enqueue_message(&NewMessage {
            sender_type: "agent".to_owned(),
            sender_id: Some(sender.id),
            receiver_agent_id: receiver.id,
            content: trim_text(&content, CONVERSATION_MESSAGE_MAX_CHARS),
        })
        .await?;
    Ok(())
}

fn choose_distinct_agents(agents: &[AgentRecord], count: usize) -> Vec<AgentRecord> {
    let mut pool: Vec<usize> = (0..agents.len()).collect();
    let target = count.min(pool.len());
    let mut selected = Vec::with_capacity(target);

    while selected.len() < target && !pool.is_empty() {
        let idx = random_index(pool.len());
        let source_idx = pool.swap_remove(idx);
        selected.push(agents[source_idx].clone());
    }

    selected
}

fn pick_random_topic() -> &'static str {
    let idx = random_index(CONVERSATION_TOPICS.len());
    CONVERSATION_TOPICS[idx]
}

fn random_duration_between(min: Duration, max: Duration) -> Duration {
    let min_ms = u64::try_from(min.as_millis()).unwrap_or(u64::MAX / 2);
    let max_ms = u64::try_from(max.as_millis()).unwrap_or(u64::MAX / 2);
    if max_ms <= min_ms {
        return Duration::from_millis(min_ms);
    }

    let span = max_ms.saturating_sub(min_ms);
    let offset = if span == 0 {
        0
    } else {
        random_u64() % span.saturating_add(1)
    };
    Duration::from_millis(min_ms.saturating_add(offset))
}

fn random_bool(chance_percent: u8) -> bool {
    let threshold = chance_percent.min(100) as u64;
    (random_u64() % 100) < threshold
}

fn random_index(len: usize) -> usize {
    if len <= 1 {
        return 0;
    }
    (random_u64() % len as u64) as usize
}

fn random_u64() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let uuid_bits = Uuid::new_v4().as_u128();
    let mixed = now ^ uuid_bits ^ (uuid_bits.rotate_left(17));
    (mixed as u64) ^ ((mixed >> 64) as u64)
}

async fn process_agent_tick(
    tick_orchestrator: AgentTickOrchestrator,
    tick_memory_service: Arc<MemoryService>,
    agent_id: Uuid,
) {
    match tick_orchestrator.run_agent_tick(agent_id, None).await {
        Ok(AgentTickOrchestratorOutcome::Executed(result)) => match result.status {
            AgentTickExecutionStatus::Applied => {
                tracing::info!(
                    agent_id = %result.agent_id,
                    tick_id = %result.tick_id,
                    event_id = ?result.event_id,
                    mood = %result.mood_label,
                    "agent tick applied"
                );

                if let Err(error) = tick_memory_service
                    .append_memory(
                        result.agent_id,
                        format!(
                            "{} (mood={}, valence={:.2}, arousal={:.2})",
                            result.action_summary,
                            result.mood_label,
                            result.valence,
                            result.arousal
                        ),
                        0.7,
                    )
                    .await
                {
                    tracing::error!(
                        agent_id = %result.agent_id,
                        error = %error,
                        "failed to append episodic memory from tick"
                    );
                }
            }
            AgentTickExecutionStatus::AgentMissing => {
                tracing::warn!(
                    agent_id = %result.agent_id,
                    tick_id = %result.tick_id,
                    "agent not found for tick"
                );
            }
        },
        Ok(AgentTickOrchestratorOutcome::SkippedBusy) => {
            tracing::debug!(agent_id = %agent_id, "skipped busy agent tick");
        }
        Ok(AgentTickOrchestratorOutcome::SkippedDuplicate) => {
            tracing::debug!(agent_id = %agent_id, "skipped duplicate agent tick");
        }
        Err(error) => {
            tracing::error!(agent_id = %agent_id, error = %error, "agent tick failed");
        }
    }
}

async fn process_agent_message(repository: &dyn AgentCoreRepository, message: &MessageRecord) {
    let process = async {
        let description = format!(
            "Agent message received: {}",
            trim_text(&message.content, MESSAGE_EVENT_DESCRIPTION_CHARS)
        );

        repository
            .append_agent_event(&NewAgentEvent {
                agent_id: Some(message.receiver_agent_id),
                event_type: "agent.message.received".to_owned(),
                description,
                payload_json: json!({
                    "message_id": message.id,
                    "sender_type": message.sender_type,
                    "sender_id": message.sender_id,
                    "receiver_agent_id": message.receiver_agent_id,
                    "content": message.content,
                }),
            })
            .await?;

        if let Some(sender_agent_id) = message.sender_id {
            let affinity_delta = message_affinity_delta(&message.content);
            let relationship = repository
                .upsert_relationship_interaction(
                    sender_agent_id,
                    message.receiver_agent_id,
                    affinity_delta,
                    &trim_text(&message.content, 280),
                    Utc::now(),
                )
                .await?;
            tracing::debug!(
                message_id = message.id,
                sender = %sender_agent_id,
                receiver = %message.receiver_agent_id,
                affinity = relationship.affinity_score,
                "relationship updated from delivered message"
            );

            repository
                .append_agent_event(&NewAgentEvent {
                    agent_id: None,
                    event_type: "agent.relationship.updated".to_owned(),
                    description: format!(
                        "Relationship updated between `{}` and `{}`",
                        relationship.agent_a, relationship.agent_b
                    ),
                    payload_json: json!({
                        "relationship_id": relationship.id,
                        "agent_a": relationship.agent_a,
                        "agent_b": relationship.agent_b,
                        "affinity_score": relationship.affinity_score,
                        "history_summary": relationship.history_summary,
                        "last_interaction_at": relationship
                            .last_interaction_at
                            .map(|value| value.to_rfc3339()),
                        "created_at": relationship.created_at.to_rfc3339(),
                        "trigger_message_id": message.id,
                    }),
                })
                .await?;
        }

        repository.mark_message_delivered(message.id).await?;
        anyhow::Result::<()>::Ok(())
    }
    .await;

    match process {
        Ok(()) => {
            tracing::info!(
                message_id = message.id,
                receiver_agent_id = %message.receiver_agent_id,
                "message delivered"
            );
        }
        Err(error) => {
            if let Err(mark_error) = repository
                .mark_message_failed(message.id, &error.to_string())
                .await
            {
                tracing::error!(
                    message_id = message.id,
                    error = %mark_error,
                    "failed to mark message as failed"
                );
            }
            tracing::error!(
                message_id = message.id,
                receiver_agent_id = %message.receiver_agent_id,
                error = %error,
                "message delivery failed"
            );
        }
    }
}

fn message_affinity_delta(content: &str) -> f32 {
    let normalized = content.to_lowercase();
    let positive = keyword_hits(
        &normalized,
        &[
            "thanks",
            "help",
            "cooperate",
            "support",
            "trust",
            "friendly",
            "great",
            "good",
            "appreciate",
        ],
    );
    let negative = keyword_hits(
        &normalized,
        &[
            "hate", "threat", "attack", "angry", "bad", "conflict", "blame", "insult", "fight",
        ],
    );

    ((positive as f32 * 0.08) - (negative as f32 * 0.1)).clamp(-0.3, 0.3)
}

fn keyword_hits(content: &str, keywords: &[&str]) -> usize {
    keywords
        .iter()
        .filter(|token| content.contains(*token))
        .count()
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
