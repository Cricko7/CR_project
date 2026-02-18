use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tokio::time::sleep;

#[derive(Debug)]
pub struct OllamaRateLimiter {
    min_interval: Duration,
    next_allowed_at: Mutex<Instant>,
}

impl OllamaRateLimiter {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            next_allowed_at: Mutex::new(Instant::now()),
        }
    }

    pub fn min_interval(&self) -> Duration {
        self.min_interval
    }

    pub async fn wait_turn(&self) {
        if self.min_interval.is_zero() {
            return;
        }

        let wait_for = {
            let mut next_allowed_at = self.next_allowed_at.lock().await;
            let now = Instant::now();
            let slot = if *next_allowed_at > now {
                *next_allowed_at
            } else {
                now
            };
            *next_allowed_at = slot + self.min_interval;
            slot.saturating_duration_since(now)
        };

        if !wait_for.is_zero() {
            sleep(wait_for).await;
        }
    }
}

pub fn shared_ollama_rate_limiter(min_interval: Duration) -> Arc<OllamaRateLimiter> {
    static SHARED_LIMITER: OnceLock<Arc<OllamaRateLimiter>> = OnceLock::new();

    let limiter = SHARED_LIMITER
        .get_or_init(|| Arc::new(OllamaRateLimiter::new(min_interval)))
        .clone();

    if limiter.min_interval() != min_interval {
        tracing::warn!(
            configured_min_interval_ms = min_interval.as_millis(),
            active_min_interval_ms = limiter.min_interval().as_millis(),
            "ollama rate limiter already initialized with another interval; active value is kept"
        );
    }

    limiter
}
