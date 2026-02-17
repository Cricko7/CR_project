use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::llm::{LlmGenerateRequest, LlmGenerateResponse, LlmPort};

#[derive(Clone)]
pub struct FallbackLlm {
    primary: Arc<dyn LlmPort>,
    fallback: Arc<dyn LlmPort>,
    primary_label: &'static str,
    fallback_label: &'static str,
}

impl FallbackLlm {
    pub fn new(
        primary: Arc<dyn LlmPort>,
        fallback: Arc<dyn LlmPort>,
        primary_label: &'static str,
        fallback_label: &'static str,
    ) -> Self {
        Self {
            primary,
            fallback,
            primary_label,
            fallback_label,
        }
    }
}

#[async_trait]
impl LlmPort for FallbackLlm {
    async fn generate(&self, request: LlmGenerateRequest) -> Result<LlmGenerateResponse> {
        match self.primary.generate(request.clone()).await {
            Ok(response) => Ok(response),
            Err(primary_error) => {
                tracing::warn!(
                    primary = self.primary_label,
                    fallback = self.fallback_label,
                    error = %primary_error,
                    "primary llm request failed, switching to fallback llm"
                );
                let response = self.fallback.generate(request).await.with_context(|| {
                    format!(
                        "{} failed and fallback {} also failed",
                        self.primary_label, self.fallback_label
                    )
                })?;
                tracing::info!(
                    primary = self.primary_label,
                    fallback = self.fallback_label,
                    model = %response.model,
                    "fallback llm request succeeded"
                );
                Ok(response)
            }
        }
    }
}
