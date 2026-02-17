use super::*;

pub(super) async fn simulation_wait_duration(
    repository: &dyn AgentCoreRepository,
    base_interval: Duration,
    worker_name: &'static str,
) -> Duration {
    let time_scale = match repository.get_time_scale().await {
        Ok(record) => sanitize_time_scale(record.time_scale, worker_name),
        Err(error) => {
            tracing::warn!(
                worker = worker_name,
                error = %error,
                fallback_time_scale = DEFAULT_SIMULATION_TIME_SCALE,
                "failed to read simulation time scale; using default"
            );
            DEFAULT_SIMULATION_TIME_SCALE
        }
    };

    let scaled = base_interval.as_secs_f64() / f64::from(time_scale);
    Duration::from_secs_f64(scaled.max(MIN_SIMULATION_SLEEP.as_secs_f64()))
}

pub(super) fn sanitize_time_scale(raw: f32, worker_name: &'static str) -> f32 {
    if !raw.is_finite() {
        tracing::warn!(
            worker = worker_name,
            received_time_scale = %raw,
            fallback_time_scale = DEFAULT_SIMULATION_TIME_SCALE,
            "received non-finite simulation time scale; using default"
        );
        return DEFAULT_SIMULATION_TIME_SCALE;
    }

    let clamped = raw.clamp(MIN_SIMULATION_TIME_SCALE, MAX_SIMULATION_TIME_SCALE);
    if (clamped - raw).abs() > f32::EPSILON {
        tracing::warn!(
            worker = worker_name,
            received_time_scale = %raw,
            applied_time_scale = %clamped,
            "simulation time scale was out of range and has been clamped"
        );
    }
    clamped
}
