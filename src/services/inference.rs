use crate::{
    config::ApplicationSettings,
    domain::{
        ChatRequest, ChatResponse, DataClassification, Message, RoutingDecision, Scenario,
        TargetType,
    },
    providers::ProviderRegistry,
};
use anyhow::Result;
use std::{collections::HashMap, sync::Arc};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Clone)]
pub struct InferenceService {
    settings: Arc<ApplicationSettings>,
    registry: Arc<ProviderRegistry>,
}

impl InferenceService {
    pub fn new(settings: ApplicationSettings, registry: Arc<ProviderRegistry>) -> Self {
        Self {
            settings: Arc::new(settings),
            registry,
        }
    }

    pub async fn run(
        &self,
        scenario: &Scenario,
        routing: &RoutingDecision,
        messages: Vec<Message>,
        allow_fallback: Option<bool>,
    ) -> Result<ChatResponse> {
        let request = ChatRequest {
            request_id: Uuid::new_v4().to_string(),
            scenario_id: scenario.id.clone(),
            messages,
            temperature: None,
            max_output_tokens: None,
            stream: false,
            metadata: HashMap::from([
                (
                    "data_classification".into(),
                    match scenario.data_classification {
                        DataClassification::LocalOnly => "local_only",
                        DataClassification::CloudAllowed => "cloud_allowed",
                    }
                    .into(),
                ),
                ("routing_rule".into(), routing.rule_id.clone()),
            ]),
        };
        info!(
            event = "request_started",
            request_id = request.request_id,
            scenario_id = scenario.id,
            target = ?routing.selected_target
        );
        let provider = self.registry.for_target(routing.selected_target)?;
        let response = provider.complete_chat(&request).await;
        if response.success {
            info!(
                event = "provider_call_completed",
                request_id = response.request_id,
                provider_id = response.provider_id,
                latency_ms = response.latency_ms
            );
            return Ok(response);
        }

        if !allow_fallback.unwrap_or(self.settings.allow_fallback) {
            return Ok(response);
        }

        for fallback_target in &self.settings.fallback_order {
            if *fallback_target == routing.selected_target {
                continue;
            }
            if *fallback_target == TargetType::Cloud
                && scenario.data_classification == DataClassification::LocalOnly
            {
                continue;
            }
            let Ok(provider) = self.registry.for_target(*fallback_target) else {
                continue;
            };
            warn!(
                event = "fallback_triggered",
                request_id = request.request_id,
                original_target = ?routing.selected_target,
                fallback_target = ?fallback_target
            );
            let mut fallback = provider.complete_chat(&request).await;
            fallback.fallback_used = true;
            if fallback.success {
                return Ok(fallback);
            }
        }
        Ok(response)
    }
}
