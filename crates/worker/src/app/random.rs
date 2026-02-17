use super::*;

pub(super) fn random_duration_between(min: Duration, max: Duration) -> Duration {
    let min_ms = u64::try_from(min.as_millis()).unwrap_or(u64::MAX / 2);
    let max_ms = u64::try_from(max.as_millis()).unwrap_or(u64::MAX / 2);
    if max_ms <= min_ms {
        return Duration::from_millis(min_ms);
    }

    let span = max_ms.saturating_sub(min_ms);
    let offset = if span == 0 {
        0
    } else {
        random_u64() % span.saturating_add(1)
    };
    Duration::from_millis(min_ms.saturating_add(offset))
}

pub(super) fn random_bool(chance_percent: u8) -> bool {
    let threshold = chance_percent.min(100) as u64;
    (random_u64() % 100) < threshold
}

pub(super) fn random_index(len: usize) -> usize {
    if len <= 1 {
        return 0;
    }
    (random_u64() % len as u64) as usize
}

pub(super) fn random_u64() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let uuid_bits = Uuid::new_v4().as_u128();
    let mixed = now ^ uuid_bits ^ (uuid_bits.rotate_left(17));
    (mixed as u64) ^ ((mixed >> 64) as u64)
}
