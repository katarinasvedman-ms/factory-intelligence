mod agentic_retrieval;
mod local_rag_openai;
mod mock;
mod openai_compatible;

use crate::{
    config::DemoSettings,
    domain::{ChatRequest, ChatResponse, ProviderCapabilities, ProviderStatus, TargetType},
};
use agentic_retrieval::AgenticRetrievalProvider;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use local_rag_openai::LocalRagOpenAiProvider;
use mock::MockProvider;
use openai_compatible::OpenAiCompatibleProvider;
use std::{collections::HashMap, sync::Arc};

#[async_trait]
pub trait InferenceProvider: Send + Sync {
    fn id(&self) -> &str;
    fn target_type(&self) -> TargetType;
    fn is_mock(&self) -> bool;
    fn capabilities(&self) -> ProviderCapabilities;
    async fn health(&self) -> ProviderStatus;
    async fn complete_chat(&self, request: &ChatRequest) -> ChatResponse;
}

#[derive(Clone)]
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn InferenceProvider>>,
    targets: HashMap<TargetType, String>,
}

impl ProviderRegistry {
    pub fn new(settings: &DemoSettings) -> Result<Self> {
        let mut providers: HashMap<String, Arc<dyn InferenceProvider>> = HashMap::new();
        let mut targets = HashMap::new();

        for (id, provider_settings) in &settings.providers {
            let provider: Arc<dyn InferenceProvider> =
                match provider_settings.provider_type.as_str() {
                    "mock" => Arc::new(MockProvider::new(id, provider_settings.clone())),
                    "openai_compatible" => Arc::new(OpenAiCompatibleProvider::new(
                        id,
                        provider_settings.clone(),
                        settings.application.request_timeout_seconds,
                    )?),
                    "agentic_retrieval" => Arc::new(AgenticRetrievalProvider::new(
                        id,
                        provider_settings.clone(),
                        settings.application.request_timeout_seconds,
                    )?),
                    "local_rag_openai" => Arc::new(LocalRagOpenAiProvider::new(
                        id,
                        provider_settings.clone(),
                        settings.application.request_timeout_seconds,
                    )?),
                    other => return Err(anyhow!("unsupported provider type: {other}")),
                };

            if provider_settings.configured() {
                let target = provider_settings.target_type;
                let replace = targets
                    .get(&target)
                    .and_then(|current| providers.get(current))
                    .is_none_or(|current| current.is_mock() && !provider.is_mock());
                if replace {
                    targets.insert(target, id.clone());
                }
            }
            providers.insert(id.clone(), provider);
        }
        Ok(Self { providers, targets })
    }

    pub fn all(&self) -> Vec<Arc<dyn InferenceProvider>> {
        self.providers.values().cloned().collect()
    }

    pub fn by_id(&self, id: &str) -> Option<Arc<dyn InferenceProvider>> {
        self.providers.get(id).cloned()
    }

    pub fn for_target(&self, target: TargetType) -> Result<Arc<dyn InferenceProvider>> {
        let id = self
            .targets
            .get(&target)
            .ok_or_else(|| anyhow!("no configured provider for target {target:?}"))?;
        self.by_id(id)
            .ok_or_else(|| anyhow!("configured provider {id} is missing"))
    }
}
