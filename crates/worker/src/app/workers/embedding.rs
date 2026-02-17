use super::*;

pub(crate) fn spawn_memory_embedding_worker(
    runtime: &mut ServiceRuntime,
    embed_interval: Duration,
    embed_batch: u32,
    embed_service: Arc<MemoryService>,
) {
    let embed_token = runtime.cancellation_token();
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
}
