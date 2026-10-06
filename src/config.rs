use crate::domain::{ProviderCapabilities, TargetType};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::{collections::HashMap, env, fs, path::PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct ApplicationSettings {
    #[serde(default = "default_target")]
    pub default_target: String,
    #[serde(default)]
    pub allow_fallback: bool,
    #[serde(default = "default_fallback")]
    pub fallback_order: Vec<TargetType>,
    #[serde(default)]
    pub log_prompt_content: bool,
    #[serde(default = "default_timeout")]
    pub request_timeout_seconds: f64,
}

fn default_target() -> String {
    "device".into()
}

fn default_fallback() -> Vec<TargetType> {
    vec![TargetType::Edge, TargetType::Device]
}

fn default_timeout() -> f64 {
    30.0
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(rename = "type", default = "default_provider_type")]
    pub provider_type: String,
    pub target_type: TargetType,
    pub endpoint_alias: String,
    pub base_url_env: Option<String>,
    pub api_key_env: Option<String>,
    pub model_env: Option<String>,
    pub agent_id_env: Option<String>,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub agent_id: Option<String>,
    pub knowledge_path: Option<String>,
    #[serde(default = "default_auth_mode")]
    pub auth_mode: String,
    #[serde(default = "default_auth_header")]
    pub auth_header: String,
    #[serde(default = "default_chat_path")]
    pub chat_path: String,
    #[serde(default = "default_health_path")]
    pub health_path: String,
    #[serde(default)]
    pub capabilities: ProviderCapabilities,
    #[serde(default = "default_agent_poll_interval")]
    pub agent_poll_interval_ms: u64,
}

fn default_provider_type() -> String {
    "openai_compatible".into()
}
fn default_auth_mode() -> String {
    "none".into()
}
fn default_auth_header() -> String {
    "api-key".into()
}
fn default_chat_path() -> String {
    "/v1/chat/completions".into()
}
fn default_health_path() -> String {
    "/v1/models".into()
}
fn default_agent_poll_interval() -> u64 {
    750
}

impl ProviderSettings {
    pub fn resolved_base_url(&self) -> Option<String> {
        self.base_url_env
            .as_ref()
            .and_then(|name| env::var(name).ok())
            .filter(|value| !value.is_empty())
            .or_else(|| self.base_url.clone())
    }

    pub fn resolved_api_key(&self) -> Option<String> {
        self.api_key_env
            .as_ref()
            .and_then(|name| env::var(name).ok())
            .filter(|value| !value.is_empty())
    }

    pub fn resolved_model(&self) -> Option<String> {
        self.model_env
            .as_ref()
            .and_then(|name| env::var(name).ok())
            .filter(|value| !value.is_empty())
            .or_else(|| self.model.clone())
    }

    pub fn resolved_agent_id(&self) -> Option<String> {
        self.agent_id_env
            .as_ref()
            .and_then(|name| env::var(name).ok())
            .filter(|value| !value.is_empty())
            .or_else(|| self.agent_id.clone())
    }

    pub fn configured(&self) -> bool {
        if !self.enabled {
            return false;
        }
        if self.provider_type == "mock" {
            return self.resolved_model().is_some();
        }
        if self.provider_type == "agentic_retrieval" {
            return self.resolved_base_url().is_some()
                && self.resolved_agent_id().is_some()
                && (self.auth_mode == "none" || self.resolved_api_key().is_some());
        }
        if self.provider_type == "local_rag_openai" && self.knowledge_path.is_none() {
            return false;
        }
        self.resolved_base_url().is_some() && self.resolved_model().is_some()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DemoSettings {
    pub application: ApplicationSettings,
    pub providers: HashMap<String, ProviderSettings>,
}

pub fn load_settings() -> Result<DemoSettings> {
    let requested = env::var("DEMO_CONFIG").ok().map(PathBuf::from);
    let path = requested
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from("config/demo.example.yaml"));
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("failed to read configuration {}", path.display()))?;
    serde_yaml::from_str(&contents)
        .with_context(|| format!("failed to parse configuration {}", path.display()))
}
