mod fallback;

use anyhow::Result;
use async_trait::async_trait;

pub use fallback::FallbackLlm;

#[derive(Debug, Clone)]
pub struct LlmGenerateRequest {
    pub system_prompt: Option<String>,
    pub user_prompt: String,
    pub temperature: Option<f32>,
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct LlmGenerateResponse {
    pub text: String,
    pub model: String,
}

#[async_trait]
pub trait LlmPort: Send + Sync {
    async fn generate(&self, request: LlmGenerateRequest) -> Result<LlmGenerateResponse>;
}
