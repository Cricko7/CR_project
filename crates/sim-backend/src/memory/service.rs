use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use uuid::Uuid;

use crate::llm::{LlmGenerateRequest, LlmPort};
use crate::memory::{
    MemoryEntryRecord, MemoryRepository, MemoryVectorStore, NewMemoryEntry, TextEmbedder,
    VectorSearchHit,
};

const DEFAULT_SUMMARY_CHARS: usize = 1200;
const DEFAULT_EVENT_CHARS: usize = 240;

#[derive(Debug, Clone)]
pub struct MemoryRecallItem {
    pub memory: MemoryEntryRecord,
    pub score: f32,
}

#[derive(Debug, Clone)]
pub struct MemoryProcessSummary {
    pub processed: u32,
    pub succeeded: u32,
    pub failed: u32,
}

#[derive(Debug, Clone)]
pub struct MemorySummaryResult {
    pub created_summary: bool,
    pub source_count: u32,
    pub summary_entry_id: Option<i64>,
}

#[derive(Clone)]
pub struct MemoryService {
    repository: Arc<dyn MemoryRepository>,
    vector_store: Arc<dyn MemoryVectorStore>,
    embedder: Arc<dyn TextEmbedder>,
    summarizer_llm: Option<Arc<dyn LlmPort>>,
    vector_size: usize,
}

impl MemoryService {
    pub fn new(
        repository: Arc<dyn MemoryRepository>,
        vector_store: Arc<dyn MemoryVectorStore>,
        embedder: Arc<dyn TextEmbedder>,
        summarizer_llm: Option<Arc<dyn LlmPort>>,
        vector_size: usize,
    ) -> Self {
        Self {
            repository,
            vector_store,
            embedder,
            summarizer_llm,
            vector_size,
        }
    }

    pub async fn append_memory(
        &self,
        agent_id: Uuid,
        content: impl Into<String>,
        importance: f32,
    ) -> Result<MemoryEntryRecord> {
        let content = trim_text(content.into().as_str(), DEFAULT_SUMMARY_CHARS);
        self.repository
            .insert_memory_entry(&NewMemoryEntry {
                agent_id,
                content,
                summary: None,
                importance: importance.clamp(0.0, 1.0),
                is_summary: false,
            })
            .await
    }

    pub async fn process_pending_embeddings(&self, limit: u32) -> Result<MemoryProcessSummary> {
        let pending = self.repository.list_pending_embeddings(limit).await?;
        let mut summary = MemoryProcessSummary {
            processed: pending.len() as u32,
            succeeded: 0,
            failed: 0,
        };

        for memory in pending {
            let text = memory.summary.clone().unwrap_or_else(|| memory.content.clone());
            let embedding_result = self.embedder.embed_document(&text).await;
            match embedding_result {
                Ok(vector) => {
                    let normalized = normalize_vector_size(vector, self.vector_size);
                    let upsert_result = self
                        .vector_store
                        .upsert_memory_vector(
                            memory.id,
                            memory.agent_id,
                            normalized,
                            memory.importance,
                            memory.is_summary,
                            memory.created_at,
                        )
                        .await;
                    match upsert_result {
                        Ok(()) => {
                            self.repository
                                .mark_embedding_done(memory.id, self.embedder.model_name())
                                .await?;
                            summary.succeeded += 1;
                        }
                        Err(error) => {
                            self.repository
                                .mark_embedding_failed(memory.id, &error.to_string())
                                .await?;
                            summary.failed += 1;
                        }
                    }
                }
                Err(error) => {
                    self.repository
                        .mark_embedding_failed(memory.id, &error.to_string())
                        .await?;
                    summary.failed += 1;
                }
            }
        }

        Ok(summary)
    }

    pub async fn recall(
        &self,
        agent_id: Uuid,
        query: &str,
        top_k: u32,
    ) -> Result<Vec<MemoryRecallItem>> {
        let query_vector = self.embedder.embed_query(query).await?;
        let query_vector = normalize_vector_size(query_vector, self.vector_size);
        let hits = self
            .vector_store
            .search_agent_memories(agent_id, query_vector, top_k)
            .await?;
        hydrate_recall_items(self.repository.as_ref(), hits).await
    }

    pub async fn summarize_overflow(
        &self,
        agent_id: Uuid,
        max_active_memories: u32,
        summary_batch_size: u32,
    ) -> Result<MemorySummaryResult> {
        let active = self.repository.count_active_memories(agent_id).await?;
        if active <= u64::from(max_active_memories) {
            return Ok(MemorySummaryResult {
                created_summary: false,
                source_count: 0,
                summary_entry_id: None,
            });
        }

        let overflow = active.saturating_sub(u64::from(max_active_memories));
        let source_count = overflow.min(u64::from(summary_batch_size)) as u32;
        let source = self
            .repository
            .list_oldest_active_memories(agent_id, source_count)
            .await?;
        if source.is_empty() {
            return Ok(MemorySummaryResult {
                created_summary: false,
                source_count: 0,
                summary_entry_id: None,
            });
        }

        let summary_text = self.build_summary_text(agent_id, &source).await;
        let average_importance = average_importance(&source);
        let summary_entry = self
            .repository
            .insert_memory_entry(&NewMemoryEntry {
                agent_id,
                content: trim_text(&summary_text, DEFAULT_SUMMARY_CHARS),
                summary: Some(trim_text(&summary_text, DEFAULT_SUMMARY_CHARS)),
                importance: average_importance,
                is_summary: true,
            })
            .await?;

        let source_ids: Vec<i64> = source.into_iter().map(|entry| entry.id).collect();
        self.repository
            .archive_memories(&source_ids, summary_entry.id)
            .await?;

        Ok(MemorySummaryResult {
            created_summary: true,
            source_count: source_ids.len() as u32,
            summary_entry_id: Some(summary_entry.id),
        })
    }

    async fn build_summary_text(&self, agent_id: Uuid, source: &[MemoryEntryRecord]) -> String {
        let deterministic = deterministic_summary(source);
        let Some(llm) = &self.summarizer_llm else {
            return deterministic;
        };

        let prompt = source
            .iter()
            .take(12)
            .map(|entry| {
                let short = trim_text(&entry.content, DEFAULT_EVENT_CHARS);
                format!(
                    "[{}] {}",
                    entry.created_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    short
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let request = LlmGenerateRequest {
            system_prompt: Some(
                "Summarize episodic memories for one autonomous agent. Keep factual, concise, and preserve key relationships/conflicts."
                    .to_owned(),
            ),
            user_prompt: format!(
                "Agent {agent_id}. Summarize these memories in 4-6 sentences:\n{prompt}"
            ),
            temperature: Some(0.2),
            max_output_tokens: Some(200),
        };

        match llm.generate(request).await {
            Ok(response) => trim_text(&response.text, DEFAULT_SUMMARY_CHARS),
            Err(error) => {
                tracing::warn!(agent_id = %agent_id, error = %error, "llm memory summarization failed, using deterministic summary");
                deterministic
            }
        }
    }
}

async fn hydrate_recall_items(
    repository: &dyn MemoryRepository,
    hits: Vec<VectorSearchHit>,
) -> Result<Vec<MemoryRecallItem>> {
    if hits.is_empty() {
        return Ok(Vec::new());
    }

    let ids: Vec<i64> = hits.iter().map(|hit| hit.memory_id).collect();
    let records = repository.list_memories_by_ids(&ids).await?;
    let lookup: HashMap<i64, MemoryEntryRecord> =
        records.into_iter().map(|record| (record.id, record)).collect();

    let mut items = Vec::with_capacity(hits.len());
    for hit in hits {
        if let Some(record) = lookup.get(&hit.memory_id) {
            items.push(MemoryRecallItem {
                memory: record.clone(),
                score: hit.score,
            });
        }
    }
    Ok(items)
}

fn normalize_vector_size(mut vector: Vec<f32>, expected_size: usize) -> Vec<f32> {
    if vector.len() > expected_size {
        vector.truncate(expected_size);
        return vector;
    }

    if vector.len() < expected_size {
        vector.resize(expected_size, 0.0);
    }

    vector
}

fn average_importance(entries: &[MemoryEntryRecord]) -> f32 {
    if entries.is_empty() {
        return 0.5;
    }
    let total: f32 = entries.iter().map(|entry| entry.importance).sum();
    (total / entries.len() as f32).clamp(0.0, 1.0)
}

fn deterministic_summary(entries: &[MemoryEntryRecord]) -> String {
    if entries.is_empty() {
        return "No memories available for summarization.".to_owned();
    }

    let mut segments = Vec::new();
    for entry in entries.iter().take(8) {
        let short = trim_text(&entry.content, DEFAULT_EVENT_CHARS);
        segments.push(format!(
            "[{}] {}",
            entry.created_at.format("%Y-%m-%d %H:%M"),
            short
        ));
    }
    format!("Summary of past memories: {}", segments.join(" | "))
}

fn trim_text(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.trim().to_owned();
    }
    input.chars().take(max_chars).collect::<String>().trim().to_owned()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use anyhow::Result;
    use async_trait::async_trait;
    use chrono::Utc;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    use crate::memory::{
        MemoryEntryRecord, MemoryRepository, MemoryVectorStore, NewMemoryEntry, SimpleHashEmbedder,
        TextEmbedder, VectorSearchHit,
    };

    use super::MemoryService;

    #[derive(Default)]
    struct InMemoryRepo {
        rows: Mutex<Vec<MemoryEntryRecord>>,
    }

    #[async_trait]
    impl MemoryRepository for InMemoryRepo {
        async fn insert_memory_entry(&self, new_entry: &NewMemoryEntry) -> Result<MemoryEntryRecord> {
            let mut rows = self.rows.lock().await;
            let record = MemoryEntryRecord {
                id: rows.len() as i64 + 1,
                agent_id: new_entry.agent_id,
                content: new_entry.content.clone(),
                summary: new_entry.summary.clone(),
                importance: new_entry.importance,
                is_summary: new_entry.is_summary,
                archived: false,
                embedding_status: "pending".to_owned(),
                created_at: Utc::now(),
            };
            rows.push(record.clone());
            Ok(record)
        }

        async fn list_pending_embeddings(&self, _limit: u32) -> Result<Vec<MemoryEntryRecord>> {
            let rows = self.rows.lock().await;
            Ok(rows
                .iter()
                .filter(|row| row.embedding_status == "pending")
                .cloned()
                .collect())
        }

        async fn mark_embedding_done(&self, memory_id: i64, _embedding_model: &str) -> Result<()> {
            let mut rows = self.rows.lock().await;
            if let Some(row) = rows.iter_mut().find(|row| row.id == memory_id) {
                row.embedding_status = "embedded".to_owned();
            }
            Ok(())
        }

        async fn mark_embedding_failed(&self, memory_id: i64, _error: &str) -> Result<()> {
            let mut rows = self.rows.lock().await;
            if let Some(row) = rows.iter_mut().find(|row| row.id == memory_id) {
                row.embedding_status = "failed".to_owned();
            }
            Ok(())
        }

        async fn list_memories_by_ids(&self, ids: &[i64]) -> Result<Vec<MemoryEntryRecord>> {
            let rows = self.rows.lock().await;
            Ok(rows
                .iter()
                .filter(|row| ids.contains(&row.id))
                .cloned()
                .collect())
        }

        async fn list_oldest_active_memories(
            &self,
            agent_id: Uuid,
            limit: u32,
        ) -> Result<Vec<MemoryEntryRecord>> {
            let rows = self.rows.lock().await;
            Ok(rows
                .iter()
                .filter(|row| row.agent_id == agent_id && !row.archived && !row.is_summary)
                .take(limit as usize)
                .cloned()
                .collect())
        }

        async fn count_active_memories(&self, agent_id: Uuid) -> Result<u64> {
            let rows = self.rows.lock().await;
            Ok(rows
                .iter()
                .filter(|row| row.agent_id == agent_id && !row.archived && !row.is_summary)
                .count() as u64)
        }

        async fn archive_memories(&self, ids: &[i64], _summarized_by_id: i64) -> Result<()> {
            let mut rows = self.rows.lock().await;
            for row in &mut *rows {
                if ids.contains(&row.id) {
                    row.archived = true;
                }
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct InMemoryVectorStore {
        hits: Mutex<Vec<VectorSearchHit>>,
    }

    #[async_trait]
    impl MemoryVectorStore for InMemoryVectorStore {
        async fn ensure_collection(&self) -> Result<()> {
            Ok(())
        }

        async fn upsert_memory_vector(
            &self,
            _memory_id: i64,
            _agent_id: Uuid,
            _vector: Vec<f32>,
            _importance: f32,
            _is_summary: bool,
            _created_at: chrono::DateTime<Utc>,
        ) -> Result<()> {
            Ok(())
        }

        async fn search_agent_memories(
            &self,
            _agent_id: Uuid,
            _query_vector: Vec<f32>,
            _top_k: u32,
        ) -> Result<Vec<VectorSearchHit>> {
            Ok(self.hits.lock().await.clone())
        }
    }

    #[tokio::test]
    async fn stores_and_recalls_memory() {
        let repo = Arc::new(InMemoryRepo::default());
        let store = Arc::new(InMemoryVectorStore::default());
        let embedder: Arc<dyn TextEmbedder> = Arc::new(SimpleHashEmbedder::new(8));
        let service = MemoryService::new(repo.clone(), store.clone(), embedder, None, 8);
        let agent_id = Uuid::new_v4();

        let first = service
            .append_memory(agent_id, "Found a treasure map", 0.9)
            .await
            .expect("memory should be stored");
        let second = service
            .append_memory(agent_id, "Argued with another agent", 0.7)
            .await
            .expect("memory should be stored");

        let process = service
            .process_pending_embeddings(10)
            .await
            .expect("processing should succeed");
        assert_eq!(process.processed, 2);
        assert_eq!(process.succeeded, 2);

        {
            let mut hits = store.hits.lock().await;
            hits.push(VectorSearchHit {
                memory_id: second.id,
                score: 0.91,
            });
            hits.push(VectorSearchHit {
                memory_id: first.id,
                score: 0.88,
            });
        }

        let recalled = service
            .recall(agent_id, "conflict", 2)
            .await
            .expect("recall should succeed");
        assert_eq!(recalled.len(), 2);
        assert_eq!(recalled[0].memory.id, second.id);
        assert_eq!(recalled[1].memory.id, first.id);
    }
}
