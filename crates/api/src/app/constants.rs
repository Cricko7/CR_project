use super::*;

pub(super) const EVENT_HUB_CAPACITY: usize = 512;
pub(super) const DEFAULT_WS_SNAPSHOT_LIMIT: u32 = 20;
pub(super) const DEFAULT_RECALL_TOP_K: u32 = 8;
pub(super) const DEFAULT_RELATIONSHIP_GRAPH_LIMIT: u32 = 200;
pub(super) const DEFAULT_INSPECTOR_LIMIT: u32 = 20;
pub(super) const DEFAULT_INTERVENTION_LIMIT: u32 = 50;
pub(super) const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);
pub(super) const RATE_LIMIT_STALE_TTL: Duration = Duration::from_secs(5 * 60);
pub(super) const MAX_RATE_LIMIT_BUCKETS: usize = 20_000;
pub(super) const MIN_PASSWORD_LENGTH: usize = 8;
