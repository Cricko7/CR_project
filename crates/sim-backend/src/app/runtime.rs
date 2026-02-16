use std::future::Future;
use std::time::Duration;

use anyhow::Context;
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

pub struct ServiceRuntime {
    service_name: String,
    shutdown_timeout: Duration,
    cancellation_token: CancellationToken,
    tasks: JoinSet<(String, anyhow::Result<()>)>,
}

impl ServiceRuntime {
    pub fn new(service_name: impl Into<String>, shutdown_timeout: Duration) -> Self {
        Self {
            service_name: service_name.into(),
            shutdown_timeout,
            cancellation_token: CancellationToken::new(),
            tasks: JoinSet::new(),
        }
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation_token.clone()
    }

    pub fn spawn<F>(&mut self, name: impl Into<String>, task: F)
    where
        F: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        let task_name = name.into();
        self.tasks.spawn(async move { (task_name, task.await) });
    }

    pub async fn run_until_shutdown(mut self) -> anyhow::Result<()> {
        let mut primary_error: Option<anyhow::Error> = None;

        if self.tasks.is_empty() {
            tracing::warn!(service = self.service_name, "runtime started without tasks");
            return Ok(());
        }

        tokio::select! {
            _ = wait_for_shutdown_signal() => {
                tracing::info!(service = self.service_name, "shutdown signal received");
            }
            task_result = self.tasks.join_next() => {
                if let Some(joined) = task_result {
                    match joined {
                        Ok((task, Ok(()))) => {
                            tracing::warn!(service = self.service_name, task, "task exited before shutdown signal");
                        }
                        Ok((task, Err(error))) => {
                            tracing::error!(service = self.service_name, task, error = %error, "task failed");
                            primary_error = Some(error.context(format!("task `{task}` failed")));
                        }
                        Err(join_error) => {
                            tracing::error!(service = self.service_name, error = %join_error, "task panicked or was cancelled");
                            primary_error = Some(anyhow::Error::new(join_error).context("task join failure"));
                        }
                    }
                }
            }
        }

        self.cancellation_token.cancel();
        tracing::info!(service = self.service_name, "cancellation requested");

        let drain_future = async {
            while let Some(joined) = self.tasks.join_next().await {
                match joined {
                    Ok((task, Ok(()))) => {
                        tracing::info!(service = self.service_name, task, "task stopped");
                    }
                    Ok((task, Err(error))) => {
                        tracing::error!(service = self.service_name, task, error = %error, "task stopped with error");
                        if primary_error.is_none() {
                            primary_error = Some(error.context(format!("task `{task}` failed during shutdown")));
                        }
                    }
                    Err(join_error) => {
                        tracing::error!(service = self.service_name, error = %join_error, "task join failed during shutdown");
                        if primary_error.is_none() {
                            primary_error = Some(
                                anyhow::Error::new(join_error)
                                    .context("task join failure during shutdown"),
                            );
                        }
                    }
                }
            }
        };

        timeout(self.shutdown_timeout, drain_future)
            .await
            .context("graceful shutdown timed out")?;

        if let Some(error) = primary_error {
            return Err(error);
        }

        tracing::info!(service = self.service_name, "service stopped cleanly");
        Ok(())
    }
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(error) => {
                tracing::warn!(error = %error, "failed to install SIGTERM handler, falling back to Ctrl+C");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }

    #[cfg(not(unix))]
    {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %error, "failed to listen for Ctrl+C");
        }
    }
}
