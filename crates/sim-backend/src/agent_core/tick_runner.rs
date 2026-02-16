use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::sync::Arc;

use tokio::sync::{Mutex, Semaphore};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TickRunOutcome<T> {
    Executed(T),
    SkippedBusy,
    SkippedDuplicate,
}

#[derive(Clone)]
pub struct AgentTickRunner {
    state: Arc<Mutex<TickRunnerState>>,
    max_history_per_agent: usize,
}

impl AgentTickRunner {
    pub fn new(max_history_per_agent: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(TickRunnerState::default())),
            max_history_per_agent,
        }
    }

    pub async fn run_tick<T, F, Fut>(
        &self,
        agent_id: &str,
        tick_id: &str,
        work: F,
    ) -> TickRunOutcome<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = T>,
    {
        let semaphore = {
            let mut state = self.state.lock().await;

            if state.has_seen_tick(agent_id, tick_id) {
                return TickRunOutcome::SkippedDuplicate;
            }

            state
                .semaphores
                .entry(agent_id.to_owned())
                .or_insert_with(|| Arc::new(Semaphore::new(1)))
                .clone()
        };

        let Ok(_permit) = semaphore.try_acquire_owned() else {
            return TickRunOutcome::SkippedBusy;
        };

        let output = work().await;

        let mut state = self.state.lock().await;
        state.record_tick(agent_id, tick_id, self.max_history_per_agent);
        TickRunOutcome::Executed(output)
    }
}

#[derive(Default)]
struct TickRunnerState {
    semaphores: HashMap<String, Arc<Semaphore>>,
    history: HashMap<String, AgentTickHistory>,
}

impl TickRunnerState {
    fn has_seen_tick(&self, agent_id: &str, tick_id: &str) -> bool {
        self.history
            .get(agent_id)
            .map(|history| history.seen.contains(tick_id))
            .unwrap_or(false)
    }

    fn record_tick(&mut self, agent_id: &str, tick_id: &str, max_history_per_agent: usize) {
        let history = self.history.entry(agent_id.to_owned()).or_default();
        history.insert(tick_id.to_owned(), max_history_per_agent);
    }
}

#[derive(Default)]
struct AgentTickHistory {
    order: VecDeque<String>,
    seen: HashSet<String>,
}

impl AgentTickHistory {
    fn insert(&mut self, tick_id: String, max_entries: usize) {
        if self.seen.contains(&tick_id) {
            return;
        }

        self.seen.insert(tick_id.clone());
        self.order.push_back(tick_id);

        while self.order.len() > max_entries {
            if let Some(evicted) = self.order.pop_front() {
                self.seen.remove(&evicted);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use tokio::time::sleep;

    use super::{AgentTickRunner, TickRunOutcome};

    #[tokio::test]
    async fn skips_second_tick_when_agent_is_busy() {
        let runner = AgentTickRunner::new(32);
        let hit_counter = Arc::new(AtomicUsize::new(0));

        let first_counter = Arc::clone(&hit_counter);
        let first = tokio::spawn({
            let runner = runner.clone();
            async move {
                runner
                    .run_tick("agent-1", "tick-1", move || async move {
                        first_counter.fetch_add(1, Ordering::SeqCst);
                        sleep(Duration::from_millis(100)).await;
                    })
                    .await
            }
        });

        sleep(Duration::from_millis(20)).await;

        let second = runner
            .run_tick("agent-1", "tick-2", {
                let second_counter = Arc::clone(&hit_counter);
                move || async move {
                    second_counter.fetch_add(1, Ordering::SeqCst);
                }
            })
            .await;

        let first_result = first.await.expect("first task should complete");
        assert!(matches!(first_result, TickRunOutcome::Executed(())));
        assert!(matches!(second, TickRunOutcome::SkippedBusy));
        assert_eq!(hit_counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn skips_duplicate_tick_id_after_success() {
        let runner = AgentTickRunner::new(32);
        let hit_counter = Arc::new(AtomicUsize::new(0));

        let first = runner
            .run_tick("agent-1", "tick-1", {
                let counter = Arc::clone(&hit_counter);
                move || async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    7usize
                }
            })
            .await;

        let second = runner
            .run_tick("agent-1", "tick-1", {
                let counter = Arc::clone(&hit_counter);
                move || async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    9usize
                }
            })
            .await;

        assert!(matches!(first, TickRunOutcome::Executed(7)));
        assert!(matches!(second, TickRunOutcome::SkippedDuplicate));
        assert_eq!(hit_counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn runs_different_agents_in_parallel() {
        let runner = AgentTickRunner::new(32);
        let hit_counter = Arc::new(AtomicUsize::new(0));

        let left = tokio::spawn({
            let runner = runner.clone();
            let counter = Arc::clone(&hit_counter);
            async move {
                runner
                    .run_tick("agent-1", "tick-1", move || async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        sleep(Duration::from_millis(50)).await;
                        "left"
                    })
                    .await
            }
        });

        let right = tokio::spawn({
            let runner = runner.clone();
            let counter = Arc::clone(&hit_counter);
            async move {
                runner
                    .run_tick("agent-2", "tick-1", move || async move {
                        counter.fetch_add(1, Ordering::SeqCst);
                        sleep(Duration::from_millis(50)).await;
                        "right"
                    })
                    .await
            }
        });

        let left_result = left.await.expect("left task should complete");
        let right_result = right.await.expect("right task should complete");

        assert!(matches!(left_result, TickRunOutcome::Executed("left")));
        assert!(matches!(right_result, TickRunOutcome::Executed("right")));
        assert_eq!(hit_counter.load(Ordering::SeqCst), 2);
    }
}
