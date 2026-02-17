use super::*;

pub(crate) fn spawn_agent_tick_worker(
    runtime: &mut ServiceRuntime,
    agent_ids: Vec<Uuid>,
    tick_interval: Duration,
    tick_concurrency: usize,
    tick_orchestrator: AgentTickOrchestrator,
    tick_memory_service: Arc<MemoryService>,
    tick_scale_repository: Arc<dyn AgentCoreRepository>,
) {
    let tick_token = runtime.cancellation_token();
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
                            let permit = match permit_pool.acquire_owned().await {
                                Ok(permit) => permit,
                                Err(error) => {
                                    tracing::error!(
                                        agent_id = %agent_id,
                                        error = %error,
                                        "failed to acquire tick worker semaphore permit"
                                    );
                                    return;
                                }
                            };
                            let _permit = permit;
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
}
