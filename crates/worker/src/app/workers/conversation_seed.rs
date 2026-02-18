use super::*;

pub(crate) fn spawn_conversation_seed_worker(
    runtime: &mut ServiceRuntime,
    conversation_repository: Arc<dyn AgentCoreRepository>,
    llm: Option<Arc<dyn LlmPort>>,
    conversation_scan_interval: Duration,
    conversation_min_interval: Duration,
    conversation_max_interval: Duration,
    conversation_agent_limit: u32,
) {
    let conversation_token = runtime.cancellation_token();
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
                        match seed_onboarding_conversation(
                            conversation_repository.as_ref(),
                            new_agent,
                            &agents,
                            llm.as_deref(),
                        )
                        .await
                        {
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
                        match seed_random_conversation(
                            conversation_repository.as_ref(),
                            &agents,
                            llm.as_deref(),
                        )
                        .await
                        {
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
}
