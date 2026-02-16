use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait TextEmbedder: Send + Sync {
    fn model_name(&self) -> &str;
    async fn embed_document(&self, text: &str) -> Result<Vec<f32>>;
    async fn embed_query(&self, text: &str) -> Result<Vec<f32>>;
}

#[derive(Clone)]
pub struct SimpleHashEmbedder {
    vector_size: usize,
}

impl SimpleHashEmbedder {
    pub fn new(vector_size: usize) -> Self {
        Self { vector_size }
    }
}

#[async_trait]
impl TextEmbedder for SimpleHashEmbedder {
    fn model_name(&self) -> &str {
        "local-hash-v1"
    }

    async fn embed_document(&self, text: &str) -> Result<Vec<f32>> {
        Ok(hash_embedding(text, self.vector_size))
    }

    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        Ok(hash_embedding(text, self.vector_size))
    }
}

fn hash_embedding(text: &str, vector_size: usize) -> Vec<f32> {
    let mut vector = vec![0.0_f32; vector_size];
    if vector_size == 0 {
        return vector;
    }

    for (token_idx, token) in text.split_whitespace().enumerate() {
        let mut hash = 2166136261_u64;
        for byte in token.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(1099511628211);
        }

        let index = (hash as usize) % vector_size;
        let signed = if token_idx % 2 == 0 { 1.0 } else { -1.0 };
        vector[index] += signed;
    }

    normalize(vector)
}

fn normalize(mut vector: Vec<f32>) -> Vec<f32> {
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}
