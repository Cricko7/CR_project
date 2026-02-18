use super::*;

pub(crate) fn spawn_memory_summarization_worker(
    runtime: &mut ServiceRuntime,
    summary_interval: Duration,
    summary_max_active: u32,
    summary_batch_size: u32,
    summary_agent_ids: Vec<Uuid>,
    summary_agent_limit: u32,
    summary_repository: Arc<dyn AgentCoreRepository>,
    summary_service: Arc<MemoryService>,
) {
    let summary_token = runtime.cancellation_token();
    runtime.spawn("memory_summarization_worker", async move {
        let mut interval = tokio::time::interval(summary_interval);
        loop {
            tokio::select! {
                _ = summary_token.cancelled() => {
                    tracing::info!("memory summarization worker received shutdown");
                    break;
                }
                _ = interval.tick() => {
                    let mut agent_ids = HashSet::new();
                    for agent_id in &summary_agent_ids {
                        agent_ids.insert(*agent_id);
                    }

                    match summary_repository.list_agents(summary_agent_limit).await {
                        Ok(agents) => {
                            for agent in agents {
                                agent_ids.insert(agent.id);
                            }
                        }
                        Err(error) => {
                            tracing::error!(
                                error = %error,
                                "memory summarization worker failed to list agents"
                            );
                        }
                    }

                    for agent_id in agent_ids {
                        match summary_service
                            .summarize_overflow(agent_id, summary_max_active, summary_batch_size)
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
                                tracing::error!(
                                    agent_id = %agent_id,
                                    error = %error,
                                    "memory summarization failed"
                                );
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    });
}
