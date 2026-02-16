use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::app::config::GeminiConfig;
use crate::llm::{LlmGenerateRequest, LlmGenerateResponse, LlmPort};

#[derive(Clone)]
pub struct GeminiClient {
    config: GeminiConfig,
    http: Client,
}

impl GeminiClient {
    pub fn new(config: GeminiConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(config.timeout)
            .build()
            .context("failed to build HTTP client for Gemini")?;

        Ok(Self { config, http })
    }

    fn endpoint(&self) -> String {
        let base = self.config.base_url.trim_end_matches('/');
        format!(
            "{base}/v1beta/models/{}:generateContent?key={}",
            self.config.model, self.config.api_key
        )
    }

    fn retry_backoff_for_attempt(&self, attempt: u32) -> Duration {
        let exponent = attempt.min(6);
        let factor = 2u32.pow(exponent);
        self.config.retry_backoff.saturating_mul(factor)
    }
}

#[async_trait]
impl LlmPort for GeminiClient {
    async fn generate(&self, request: LlmGenerateRequest) -> Result<LlmGenerateResponse> {
        let endpoint = self.endpoint();
        let payload = GeminiGenerateRequest::from(request);
        let mut last_error: Option<anyhow::Error> = None;

        for attempt in 0..=self.config.max_retries {
            let response = self.http.post(&endpoint).json(&payload).send().await;

            match response {
                Ok(http_response) if http_response.status().is_success() => {
                    let body: GeminiGenerateResponse = http_response
                        .json()
                        .await
                        .context("failed to deserialize Gemini response")?;
                    let text = extract_candidate_text(&body)
                        .ok_or_else(|| anyhow!("Gemini response did not contain text candidate"))?;

                    return Ok(LlmGenerateResponse {
                        text,
                        model: self.config.model.clone(),
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
                            "gemini request retry scheduled due to retryable response"
                        );
                        sleep(backoff).await;
                        continue;
                    }

                    bail!(
                        "Gemini request failed: status={} body={}",
                        status,
                        error_body
                    );
                }
                Err(error) => {
                    let wrapped = anyhow!(error).context("Gemini HTTP request failed");
                    if attempt < self.config.max_retries {
                        let backoff = self.retry_backoff_for_attempt(attempt);
                        tracing::warn!(
                            attempt,
                            backoff_ms = backoff.as_millis(),
                            error = %wrapped,
                            "gemini request retry scheduled after transport failure"
                        );
                        last_error = Some(wrapped);
                        sleep(backoff).await;
                        continue;
                    }
                    return Err(wrapped);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("Gemini request failed with unknown error")))
    }
}

fn should_retry_status(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn extract_candidate_text(response: &GeminiGenerateResponse) -> Option<String> {
    let candidate = response.candidates.first()?;
    let mut out = String::new();

    for part in &candidate.content.parts {
        if let Some(text) = &part.text {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(text.trim());
        }
    }

    if out.trim().is_empty() {
        None
    } else {
        Some(out)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiGenerateRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiSystemInstruction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GeminiGenerationConfig>,
}

impl From<LlmGenerateRequest> for GeminiGenerateRequest {
    fn from(value: LlmGenerateRequest) -> Self {
        let system_instruction = value
            .system_prompt
            .map(|system_prompt| GeminiSystemInstruction {
                parts: vec![GeminiPart {
                    text: Some(system_prompt),
                }],
            });

        let generation_config = if value.temperature.is_some() || value.max_output_tokens.is_some()
        {
            Some(GeminiGenerationConfig {
                temperature: value.temperature,
                max_output_tokens: value.max_output_tokens,
            })
        } else {
            None
        };

        Self {
            contents: vec![GeminiContent {
                role: "user".to_owned(),
                parts: vec![GeminiPart {
                    text: Some(value.user_prompt),
                }],
            }],
            system_instruction,
            generation_config,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiSystemInstruction {
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiGenerateResponse {
    candidates: Vec<GeminiCandidate>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiCandidate {
    content: GeminiContent,
}

#[cfg(test)]
mod tests {
    use super::{extract_candidate_text, should_retry_status};
    use reqwest::StatusCode;
    use serde_json::json;

    use super::GeminiGenerateResponse;

    #[test]
    fn retries_on_429_and_5xx_only() {
        assert!(should_retry_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(should_retry_status(StatusCode::BAD_GATEWAY));
        assert!(!should_retry_status(StatusCode::BAD_REQUEST));
        assert!(!should_retry_status(StatusCode::UNAUTHORIZED));
    }

    #[test]
    fn extracts_text_from_first_candidate() {
        let response: GeminiGenerateResponse = serde_json::from_value(json!({
            "candidates": [
                {
                    "content": {
                        "role": "model",
                        "parts": [{ "text": "hello" }, { "text": "world" }]
                    }
                }
            ]
        }))
        .expect("response should deserialize");

        let text = extract_candidate_text(&response).expect("text should be extracted");
        assert_eq!(text, "hello\nworld");
    }
}
