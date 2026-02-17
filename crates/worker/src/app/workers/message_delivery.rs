use super::*;

pub(crate) fn spawn_message_delivery_worker(
    runtime: &mut ServiceRuntime,
    message_interval: Duration,
    message_batch_size: u32,
    message_repository: Arc<dyn AgentCoreRepository>,
) {
    let message_token = runtime.cancellation_token();
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
}
