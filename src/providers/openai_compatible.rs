use super::InferenceProvider;
use crate::{
    config::ProviderSettings,
    domain::{
        ChatRequest, ChatResponse, ProviderCapabilities, ProviderError, ProviderStatus, TargetType,
    },
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use reqwest::{Client, StatusCode, header::HeaderMap};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub struct OpenAiCompatibleProvider {
    id: String,
    settings: ProviderSettings,
    client: Client,
}

impl OpenAiCompatibleProvider {
    pub fn new(id: &str, settings: ProviderSettings, timeout_seconds: f64) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs_f64(timeout_seconds))
            .build()
            .context("failed to build provider HTTP client")?;
        Ok(Self {
            id: id.into(),
            settings,
            client,
        })
    }

    fn url(&self, path: &str) -> Option<String> {
        self.settings.resolved_base_url().map(|base| {
            format!(
                "{}/{}",
                base.trim_end_matches('/'),
                path.trim_start_matches('/')
            )
        })
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(credential) = self.settings.resolved_api_key() {
            match self.settings.auth_mode.as_str() {
                "api_key" => {
                    if let (Ok(name), Ok(value)) = (
                        self.settings
                            .auth_header
                            .parse::<reqwest::header::HeaderName>(),
                        credential.parse(),
                    ) {
                        headers.insert(name, value);
                    }
                }
                "bearer" => {
                    if let Ok(value) = format!("Bearer {credential}").parse() {
                        headers.insert(reqwest::header::AUTHORIZATION, value);
                    }
                }
                _ => {}
            }
        }
        headers
    }

    fn error_response(
        &self,
        request: &ChatRequest,
        started: Instant,
        code: &str,
        message: String,
        retryable: bool,
    ) -> ChatResponse {
        ChatResponse {
            request_id: request.request_id.clone(),
            provider_id: self.id.clone(),
            target_type: self.settings.target_type,
            model_id: self
                .settings
                .resolved_model()
                .unwrap_or_else(|| "unconfigured".into()),
            endpoint_alias: self.settings.endpoint_alias.clone(),
            content: String::new(),
            finish_reason: None,
            input_tokens: None,
            output_tokens: None,
            latency_ms: started.elapsed().as_millis().max(1) as u64,
            fallback_used: false,
            success: false,
            timestamp: Utc::now(),
            confidence: None,
            raw_response_logged: false,
            mock: false,
            agentic_evidence: None,
            error: Some(ProviderError {
                code: code.into(),
                message,
                retryable,
            }),
        }
    }
}

#[async_trait]
impl InferenceProvider for OpenAiCompatibleProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn target_type(&self) -> TargetType {
        self.settings.target_type
    }

    fn is_mock(&self) -> bool {
        false
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.settings.capabilities.clone()
    }

    async fn health(&self) -> ProviderStatus {
        if !self.settings.configured() {
            return ProviderStatus {
                provider_id: self.id.clone(),
                configured: false,
                reachable: false,
                authenticated: None,
                model_available: None,
                last_check_timestamp: Utc::now(),
                message: "Provider is disabled or missing endpoint/model configuration".into(),
            };
        }
        let Some(url) = self.url(&self.settings.health_path) else {
            unreachable!();
        };
        match self.client.get(url).headers(self.headers()).send().await {
            Ok(response) => {
                let status = response.status();
                ProviderStatus {
                    provider_id: self.id.clone(),
                    configured: true,
                    reachable: !status.is_server_error(),
                    authenticated: Some(!matches!(
                        status,
                        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
                    )),
                    model_available: status.is_success().then_some(true),
                    last_check_timestamp: Utc::now(),
                    message: format!("Health endpoint returned HTTP {}", status.as_u16()),
                }
            }
            Err(error) => ProviderStatus {
                provider_id: self.id.clone(),
                configured: true,
                reachable: false,
                authenticated: None,
                model_available: None,
                last_check_timestamp: Utc::now(),
                message: if error.is_timeout() {
                    "Endpoint unreachable: request timed out".into()
                } else if error.is_connect() {
                    "Endpoint unreachable: connection failed".into()
                } else {
                    "Endpoint unreachable: request failed".into()
                },
            },
        }
    }

    async fn complete_chat(&self, request: &ChatRequest) -> ChatResponse {
        let started = Instant::now();
        let Some(url) = self.url(&self.settings.chat_path) else {
            return self.error_response(
                request,
                started,
                "unconfigured_provider",
                "Provider has no configured base URL".into(),
                false,
            );
        };
        let model = self.settings.resolved_model().unwrap_or_default();
        let mut payload = json!({
            "model": model,
            "messages": request.messages,
            "stream": request.stream
        });
        if let Some(temperature) = request.temperature {
            payload["temperature"] = json!(temperature);
        }
        if let Some(max_tokens) = request.max_output_tokens {
            payload["max_tokens"] = json!(max_tokens);
        }

        let response = match self
            .client
            .post(url)
            .headers(self.headers())
            .json(&payload)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                let timeout = error.is_timeout();
                return self.error_response(
                    request,
                    started,
                    if timeout {
                        "timeout"
                    } else {
                        "provider_unreachable"
                    },
                    if timeout {
                        "Provider request timed out".into()
                    } else {
                        format!("Provider call failed: {error}")
                    },
                    true,
                );
            }
        };

        let status = response.status();
        if !status.is_success() {
            return self.error_response(
                request,
                started,
                if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                    "authentication_error"
                } else {
                    "provider_http_error"
                },
                format!("Provider returned HTTP {}", status.as_u16()),
                status.is_server_error(),
            );
        }

        let body: Value = match response.json().await {
            Ok(body) => body,
            Err(error) => {
                return self.error_response(
                    request,
                    started,
                    "invalid_provider_response",
                    format!("Provider returned invalid JSON: {error}"),
                    false,
                );
            }
        };
        let content = body
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str);
        let Some(content) = content else {
            return self.error_response(
                request,
                started,
                "invalid_provider_response",
                "Provider response did not contain choices[0].message.content".into(),
                false,
            );
        };
        ChatResponse {
            request_id: request.request_id.clone(),
            provider_id: self.id.clone(),
            target_type: self.settings.target_type,
            model_id: body
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or(&model)
                .into(),
            endpoint_alias: self.settings.endpoint_alias.clone(),
            content: content.into(),
            finish_reason: body
                .pointer("/choices/0/finish_reason")
                .and_then(Value::as_str)
                .map(str::to_owned),
            input_tokens: body.pointer("/usage/prompt_tokens").and_then(Value::as_u64),
            output_tokens: body
                .pointer("/usage/completion_tokens")
                .and_then(Value::as_u64),
            latency_ms: started.elapsed().as_millis().max(1) as u64,
            fallback_used: false,
            success: true,
            timestamp: Utc::now(),
            confidence: None,
            raw_response_logged: false,
            mock: false,
            agentic_evidence: None,
            error: None,
        }
    }
}
