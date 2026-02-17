use super::*;

pub(super) struct IpRateLimiter {
    requests_per_minute: u32,
    buckets: Arc<Mutex<HashMap<String, RateLimitBucket>>>,
}

#[derive(Clone, Copy)]
struct RateLimitBucket {
    window_started: Instant,
    request_count: u32,
}

impl IpRateLimiter {
    pub(super) fn new(requests_per_minute: u32) -> Self {
        Self {
            requests_per_minute,
            buckets: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    pub(super) async fn allow(&self, ip: &str) -> bool {
        let now = Instant::now();
        let mut buckets = self.buckets.lock().await;

        if buckets.len() > MAX_RATE_LIMIT_BUCKETS {
            buckets.retain(|_, bucket| {
                now.duration_since(bucket.window_started) <= RATE_LIMIT_STALE_TTL
            });
        }

        let bucket = buckets.entry(ip.to_owned()).or_insert(RateLimitBucket {
            window_started: now,
            request_count: 0,
        });

        if now.duration_since(bucket.window_started) >= RATE_LIMIT_WINDOW {
            bucket.window_started = now;
            bucket.request_count = 0;
        }

        if bucket.request_count >= self.requests_per_minute {
            return false;
        }

        bucket.request_count += 1;
        true
    }
}
