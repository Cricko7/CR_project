use super::*;

pub(super) const MESSAGE_CLAIM_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) const MESSAGE_EVENT_DESCRIPTION_CHARS: usize = 200;
pub(super) const MIN_SIMULATION_SLEEP: Duration = Duration::from_millis(100);
pub(super) const CONVERSATION_MESSAGE_MAX_CHARS: usize = 280;
pub(super) const MEMORY_SUMMARY_AGENT_SCAN_LIMIT: u32 = 1000;
pub(super) const CONVERSATION_TOPICS: &[&str] = &[
    "how to coordinate the next exploration step",
    "sharing quick status updates",
    "how to split responsibilities efficiently",
    "who can help with the next risky move",
    "a short plan for safer cooperation",
];
