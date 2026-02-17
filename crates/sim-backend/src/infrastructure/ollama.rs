use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::app::config::OllamaConfig;
use crate::llm::{LlmGenerateRequest, LlmGenerateResponse, LlmPort};

#[derive(Clone)]
pub struct OllamaClient {
    config: OllamaConfig,
    http: Client,
}

impl OllamaClient {
    pub fn new(config: OllamaConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(config.timeout)
            .build()
            .context("failed to build HTTP client for Ollama")?;
        Ok(Self { config, http })
    }

    fn endpoint(&self) -> String {
        let base = self.config.base_url.trim_end_matches('/');
        format!("{base}/api/generate")
    }

    fn retry_backoff_for_attempt(&self, attempt: u32) -> Duration {
        let exponent = attempt.min(6);
        let factor = 2u32.pow(exponent);
        self.config.retry_backoff.saturating_mul(factor)
    }
}

#[async_trait]
impl LlmPort for OllamaClient {
    async fn generate(&self, request: LlmGenerateRequest) -> Result<LlmGenerateResponse> {
        let endpoint = self.endpoint();
        let payload = OllamaGenerateRequest::from_llm_request(request, self.config.model.clone());
        let mut last_error: Option<anyhow::Error> = None;

        for attempt in 0..=self.config.max_retries {
            let response = self.http.post(&endpoint).json(&payload).send().await;

            match response {
                Ok(http_response) if http_response.status().is_success() => {
                    let body: OllamaGenerateResponse = http_response
                        .json()
                        .await
                        .context("failed to deserialize Ollama response")?;
                    if let Some(error_message) = body.error.as_deref() {
                        if !error_message.trim().is_empty() {
                            bail!("Ollama request failed: {}", error_message.trim());
                        }
                    }
                    let text = body
                        .response
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned)
                        .ok_or_else(|| anyhow!("Ollama response did not contain generated text"))?;

                    return Ok(LlmGenerateResponse {
                        text,
                        model: body.model.unwrap_or_else(|| self.config.model.clone()),
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
                            "ollama request retry scheduled due to retryable response"
                        );
                        sleep(backoff).await;
                        continue;
                    }

                    bail!(
                        "Ollama request failed: status={} body={}",
                        status,
                        error_body
                    );
                }
                Err(error) => {
                    let wrapped = anyhow!(error).context("Ollama HTTP request failed");
                    if attempt < self.config.max_retries {
                        let backoff = self.retry_backoff_for_attempt(attempt);
                        tracing::warn!(
                            attempt,
                            backoff_ms = backoff.as_millis(),
                            error = %wrapped,
                            "ollama request retry scheduled after transport failure"
                        );
                        last_error = Some(wrapped);
                        sleep(backoff).await;
                        continue;
                    }
                    return Err(wrapped);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("Ollama request failed with unknown error")))
    }
}

fn should_retry_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

#[derive(Debug, Serialize)]
struct OllamaGenerateRequest {
    model: String,
    prompt: String,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaGenerateOptions>,
}

impl OllamaGenerateRequest {
    fn from_llm_request(value: LlmGenerateRequest, model: String) -> Self {
        let options = if value.temperature.is_some() || value.max_output_tokens.is_some() {
            Some(OllamaGenerateOptions {
                temperature: value.temperature,
                num_predict: value.max_output_tokens,
            })
        } else {
            None
        };

        Self {
            model,
            prompt: value.user_prompt,
            stream: false,
            system: value.system_prompt,
            options,
        }
    }
}

#[derive(Debug, Serialize)]
struct OllamaGenerateOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct OllamaGenerateResponse {
    model: Option<String>,
    response: Option<String>,
    error: Option<String>,
}

#[cfg(test)]
mod tests {
    use reqwest::StatusCode;

    use super::should_retry_status;

    #[test]
    fn retries_on_429_and_5xx_only() {
        assert!(should_retry_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(should_retry_status(StatusCode::BAD_GATEWAY));
        assert!(!should_retry_status(StatusCode::BAD_REQUEST));
        assert!(!should_retry_status(StatusCode::UNAUTHORIZED));
    }
}
