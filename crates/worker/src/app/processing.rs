use super::*;

pub(super) async fn process_agent_tick(
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

pub(super) async fn process_agent_message(
    repository: &dyn AgentCoreRepository,
    message: &MessageRecord,
) {
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

pub(super) fn message_affinity_delta(content: &str) -> f32 {
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

pub(super) fn keyword_hits(content: &str, keywords: &[&str]) -> usize {
    keywords
        .iter()
        .filter(|token| content.contains(*token))
        .count()
}

pub(super) fn trim_text(input: &str, max_chars: usize) -> String {
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
