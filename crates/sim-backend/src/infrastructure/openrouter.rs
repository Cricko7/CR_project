use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::sleep;

use crate::app::config::OpenRouterConfig;
use crate::infrastructure::openrouter_rate_limiter::{
    OpenRouterRateLimiter, shared_openrouter_rate_limiter,
};
use crate::llm::{LlmGenerateRequest, LlmGenerateResponse, LlmPort};

#[derive(Clone)]
pub struct OpenRouterClient {
    config: OpenRouterConfig,
    http: Client,
    rate_limiter: Arc<OpenRouterRateLimiter>,
}

impl OpenRouterClient {
    pub fn new(config: OpenRouterConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(config.timeout)
            .build()
            .context("failed to build HTTP client for OpenRouter")?;
        let rate_limiter = shared_openrouter_rate_limiter(config.min_request_interval);
        Ok(Self {
            config,
            http,
            rate_limiter,
        })
    }

    fn endpoint(&self) -> String {
        let base = self.config.base_url.trim_end_matches('/');
        format!("{base}/chat/completions")
    }

    fn retry_backoff_for_attempt(&self, attempt: u32) -> Duration {
        let exponent = attempt.min(6);
        let factor = 2u32.pow(exponent);
        self.config.retry_backoff.saturating_mul(factor)
    }
}

#[async_trait]
impl LlmPort for OpenRouterClient {
    async fn generate(&self, request: LlmGenerateRequest) -> Result<LlmGenerateResponse> {
        let endpoint = self.endpoint();
        let payload = OpenRouterChatCompletionRequest::from_llm_request(
            request,
            self.config.model.clone(),
            self.config.reasoning_enabled,
        );
        let mut last_error: Option<anyhow::Error> = None;

        for attempt in 0..=self.config.max_retries {
            self.rate_limiter.wait_turn().await;
            let response = self
                .http
                .post(&endpoint)
                .bearer_auth(&self.config.api_key)
                .json(&payload)
                .send()
                .await;

            match response {
                Ok(http_response) if http_response.status().is_success() => {
                    let body: OpenRouterChatCompletionResponse = http_response
                        .json()
                        .await
                        .context("failed to deserialize OpenRouter response")?;
                    let text = extract_text(&body).ok_or_else(|| {
                        anyhow!("OpenRouter response did not contain assistant text")
                    })?;
                    return Ok(LlmGenerateResponse {
                        text,
                        model: body.model,
                    });
                }
                Ok(http_response) => {
                    let status = http_response.status();
                    let error_body = http_response
                        .text()
                        .await
                        .unwrap_or_else(|_| "<failed to read body>".to_owned());

                    if should_retry_status(status) && attempt < self.config.max_retries {
                        let backoff = self.retry_backoff_for_attempt(attempt);
                        tracing::warn!(
                            attempt,
                            status = %status,
                            backoff_ms = backoff.as_millis(),
                            "openrouter request retry scheduled due to retryable response"
                        );
                        sleep(backoff).await;
                        continue;
                    }

                    bail!(
                        "OpenRouter request failed: status={} body={}",
                        status,
                        error_body
                    );
                }
                Err(error) => {
                    let wrapped = anyhow!(error).context("OpenRouter HTTP request failed");
                    if attempt < self.config.max_retries {
                        let backoff = self.retry_backoff_for_attempt(attempt);
                        tracing::warn!(
                            attempt,
                            backoff_ms = backoff.as_millis(),
                            error = %wrapped,
                            "openrouter request retry scheduled after transport failure"
                        );
                        last_error = Some(wrapped);
                        sleep(backoff).await;
                        continue;
                    }
                    return Err(wrapped);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("OpenRouter request failed with unknown error")))
    }
}

fn should_retry_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn extract_text(response: &OpenRouterChatCompletionResponse) -> Option<String> {
    let choice = response.choices.first()?;
    let content = choice.message.content.as_ref()?;
    match content {
        OpenRouterMessageContent::Text(text) => {
            if text.trim().is_empty() {
                None
            } else {
                Some(text.trim().to_owned())
            }
        }
        OpenRouterMessageContent::Parts(parts) => {
            let text = parts
                .iter()
                .filter_map(|part| part.text.as_ref())
                .map(|part| part.trim())
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            if text.trim().is_empty() {
                None
            } else {
                Some(text)
            }
        }
    }
}

#[derive(Debug, Serialize)]
struct OpenRouterChatCompletionRequest {
    model: String,
    messages: Vec<OpenRouterRequestMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<OpenRouterReasoningConfig>,
}

impl OpenRouterChatCompletionRequest {
    fn from_llm_request(value: LlmGenerateRequest, model: String, reasoning_enabled: bool) -> Self {
        let mut messages = Vec::with_capacity(2);
        if let Some(system_prompt) = value.system_prompt {
            messages.push(OpenRouterRequestMessage {
                role: "system".to_owned(),
                content: system_prompt,
            });
        }
        messages.push(OpenRouterRequestMessage {
            role: "user".to_owned(),
            content: value.user_prompt,
        });

        Self {
            model,
            messages,
            temperature: value.temperature,
            max_tokens: value.max_output_tokens,
            reasoning: reasoning_enabled.then_some(OpenRouterReasoningConfig { enabled: true }),
        }
    }
}

#[derive(Debug, Serialize)]
struct OpenRouterRequestMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct OpenRouterReasoningConfig {
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct OpenRouterChatCompletionResponse {
    model: String,
    choices: Vec<OpenRouterChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterChoice {
    message: OpenRouterResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OpenRouterResponseMessage {
    content: Option<OpenRouterMessageContent>,
    #[allow(dead_code)]
    #[serde(default)]
    reasoning_details: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OpenRouterMessageContent {
    Text(String),
    Parts(Vec<OpenRouterContentPart>),
}

#[derive(Debug, Deserialize)]
struct OpenRouterContentPart {
    #[serde(default)]
    text: Option<String>,
}

#[cfg(test)]
mod tests {
    use reqwest::StatusCode;
    use serde_json::json;

    use super::{OpenRouterChatCompletionResponse, extract_text, should_retry_status};

    #[test]
    fn retries_on_429_and_5xx_only() {
        assert!(should_retry_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(should_retry_status(StatusCode::BAD_GATEWAY));
        assert!(!should_retry_status(StatusCode::BAD_REQUEST));
        assert!(!should_retry_status(StatusCode::UNAUTHORIZED));
    }

    #[test]
    fn extracts_plain_text_message() {
        let response: OpenRouterChatCompletionResponse = serde_json::from_value(json!({
            "model": "openai/gpt-oss-120b:free",
            "choices": [
                {
                    "message": {
                        "content": "hello from openrouter"
                    }
                }
            ]
        }))
        .expect("response should deserialize");

        let text = extract_text(&response).expect("text should be extracted");
        assert_eq!(text, "hello from openrouter");
    }
}
