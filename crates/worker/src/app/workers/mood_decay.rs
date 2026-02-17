use super::*;

pub(crate) fn spawn_mood_decay_worker(
    runtime: &mut ServiceRuntime,
    mood_decay_interval: Duration,
    mood_decay_step: f32,
    mood_decay_repository: Arc<PostgresAgentCoreRepository>,
) {
    let mood_token = runtime.cancellation_token();
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
}
