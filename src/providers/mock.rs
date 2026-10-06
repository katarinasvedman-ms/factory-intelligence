use super::InferenceProvider;
use crate::{
    config::ProviderSettings,
    domain::{ChatRequest, ChatResponse, ProviderCapabilities, ProviderStatus, TargetType},
};
use async_trait::async_trait;
use chrono::Utc;
use std::time::Instant;

pub struct MockProvider {
    id: String,
    settings: ProviderSettings,
}

impl MockProvider {
    pub fn new(id: &str, settings: ProviderSettings) -> Self {
        Self {
            id: id.into(),
            settings,
        }
    }
}

#[async_trait]
impl InferenceProvider for MockProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn target_type(&self) -> TargetType {
        self.settings.target_type
    }

    fn is_mock(&self) -> bool {
        true
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.settings.capabilities.clone()
    }

    async fn health(&self) -> ProviderStatus {
        ProviderStatus {
            provider_id: self.id.clone(),
            configured: self.settings.configured(),
            reachable: self.settings.enabled,
            authenticated: None,
            model_available: Some(self.settings.enabled),
            last_check_timestamp: Utc::now(),
            message: if self.settings.enabled {
                "Mock provider ready".into()
            } else {
                "Mock provider disabled".into()
            },
        }
    }

    async fn complete_chat(&self, request: &ChatRequest) -> ChatResponse {
        let started = Instant::now();
        let prompt = request
            .messages
            .last()
            .map(|message| message.content.as_str())
            .unwrap_or_default();
        ChatResponse {
            request_id: request.request_id.clone(),
            provider_id: self.id.clone(),
            target_type: self.settings.target_type,
            model_id: self
                .settings
                .resolved_model()
                .unwrap_or_else(|| "mock-factory-model".into()),
            endpoint_alias: self.settings.endpoint_alias.clone(),
            content: format!(
                "SIMULATED RESPONSE — advisory only. For scenario '{}', inspect the affected \
                 bearing, verify sensor readings, and follow the site's approved maintenance \
                 procedure. Input summary: {}",
                request.scenario_id,
                prompt.chars().take(180).collect::<String>()
            ),
            finish_reason: Some("stop".into()),
            input_tokens: None,
            output_tokens: None,
            latency_ms: started.elapsed().as_millis().max(1) as u64,
            fallback_used: false,
            success: true,
            timestamp: Utc::now(),
            confidence: Some(0.72),
            raw_response_logged: false,
            mock: true,
            agentic_evidence: None,
            error: None,
        }
    }
}
