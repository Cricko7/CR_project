mod embedder;
mod repository;
mod service;
mod vector_store;

pub use embedder::{SimpleHashEmbedder, TextEmbedder};
pub use repository::{
    EmbeddingFailureDisposition, MemoryEntryRecord, MemoryRepository, NewMemoryEntry,
};
pub use service::{MemoryProcessSummary, MemoryRecallItem, MemoryService, MemorySummaryResult};
pub use vector_store::{MemoryVectorStore, VectorSearchHit};
