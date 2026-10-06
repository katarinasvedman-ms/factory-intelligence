use super::InferenceProvider;
use crate::{
    config::ProviderSettings,
    domain::{
        AgenticRunEvidence, AgenticStepEvidence, ChatRequest, ChatResponse, KnowledgeCitation,
        ProviderCapabilities, ProviderError, ProviderStatus, TargetType,
    },
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use reqwest::{
    Client, StatusCode,
    header::{AUTHORIZATION, HeaderMap},
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tokio::time::sleep;

pub struct AgenticRetrievalProvider {
    id: String,
    settings: ProviderSettings,
    client: Client,
    timeout: Duration,
}

impl AgenticRetrievalProvider {
    pub fn new(id: &str, settings: ProviderSettings, timeout_seconds: f64) -> Result<Self> {
        let timeout = Duration::from_secs_f64(timeout_seconds);
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .context("failed to build Agentic Retrieval HTTP client")?;
        Ok(Self {
            id: id.into(),
            settings,
            client,
            timeout,
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
        if let Some(credential) = self.settings.resolved_api_key()
            && self.settings.auth_mode == "bearer"
            && let Ok(value) = format!("Bearer {credential}").parse()
        {
            headers.insert(AUTHORIZATION, value);
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
                .resolved_agent_id()
                .unwrap_or_else(|| "unconfigured-agent".into()),
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

    async fn post_json(&self, path: &str, body: Value) -> Result<Value, String> {
        let url = self
            .url(path)
            .ok_or_else(|| "Agentic Retrieval provider has no base URL".to_string())?;
        let response = self
            .client
            .post(url)
            .headers(self.headers())
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("Agentic Retrieval request failed: {error}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!(
                "Agentic Retrieval returned HTTP {}",
                status.as_u16()
            ));
        }
        response
            .json()
            .await
            .map_err(|error| format!("Agentic Retrieval returned invalid JSON: {error}"))
    }

    async fn get_json(&self, path: &str) -> Result<Value, String> {
        let url = self
            .url(path)
            .ok_or_else(|| "Agentic Retrieval provider has no base URL".to_string())?;
        let response = self
            .client
            .get(url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|error| format!("Agentic Retrieval request failed: {error}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!(
                "Agentic Retrieval returned HTTP {}",
                status.as_u16()
            ));
        }
        response
            .json()
            .await
            .map_err(|error| format!("Agentic Retrieval returned invalid JSON: {error}"))
    }

    fn assistant_content(messages: &Value) -> Option<String> {
        messages
            .get("data")?
            .as_array()?
            .iter()
            .rev()
            .find(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
            .and_then(|blocks| {
                blocks.iter().find_map(|block| {
                    block
                        .pointer("/text/value")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
            })
    }

    fn citations(messages: &Value) -> Vec<KnowledgeCitation> {
        let mut citations = Vec::new();
        let Some(data) = messages.get("data").and_then(Value::as_array) else {
            return citations;
        };
        for annotation in data
            .iter()
            .filter_map(|message| message.get("content").and_then(Value::as_array))
            .flatten()
            .filter_map(|block| block.pointer("/text/annotations").and_then(Value::as_array))
            .flatten()
        {
            citations.push(KnowledgeCitation {
                title: annotation
                    .get("title")
                    .or_else(|| annotation.pointer("/file_citation/title"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                source: annotation
                    .get("url")
                    .or_else(|| annotation.get("source"))
                    .or_else(|| annotation.pointer("/file_citation/file_id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                excerpt: annotation
                    .get("text")
                    .or_else(|| annotation.get("quote"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            });
        }
        citations
    }

    fn steps(value: &Value) -> Vec<AgenticStepEvidence> {
        let mut evidence = Vec::new();
        let Some(steps) = value.get("data").and_then(Value::as_array) else {
            return evidence;
        };
        for step in steps {
            let step_type = step
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned();
            if let Some(tool_calls) = step
                .pointer("/step_details/tool_calls")
                .and_then(Value::as_array)
            {
                for tool in tool_calls {
                    evidence.push(AgenticStepEvidence {
                        step_type: step_type.clone(),
                        tool_name: tool.get("name").and_then(Value::as_str).map(str::to_owned),
                        details: tool.clone(),
                    });
                }
            } else {
                evidence.push(AgenticStepEvidence {
                    step_type,
                    tool_name: step
                        .pointer("/step_details/mcp_call/mcp_server")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    details: step.get("step_details").cloned().unwrap_or(Value::Null),
                });
            }
        }
        evidence
    }

    fn step_citations(steps: &[AgenticStepEvidence]) -> Vec<KnowledgeCitation> {
        fn visit(value: &Value, citations: &mut Vec<KnowledgeCitation>) {
            match value {
                Value::Array(items) => {
                    for item in items {
                        visit(item, citations);
                    }
                }
                Value::Object(map) => {
                    let title = map
                        .get("title")
                        .or_else(|| map.get("document_title"))
                        .and_then(Value::as_str);
                    let source = map
                        .get("source")
                        .or_else(|| map.get("url"))
                        .or_else(|| map.get("uri"))
                        .or_else(|| map.get("file_name"))
                        .and_then(Value::as_str);
                    let excerpt = map
                        .get("excerpt")
                        .or_else(|| map.get("text"))
                        .or_else(|| map.get("content"))
                        .and_then(Value::as_str);
                    if title.is_some() || source.is_some() {
                        citations.push(KnowledgeCitation {
                            title: title.map(str::to_owned),
                            source: source.map(str::to_owned),
                            excerpt: excerpt.map(str::to_owned),
                        });
                    }
                    for child in map.values() {
                        visit(child, citations);
                    }
                }
                Value::String(text)
                    if text.trim_start().starts_with('{') || text.trim_start().starts_with('[') =>
                {
                    if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                        visit(&parsed, citations);
                    }
                }
                _ => {}
            }
        }

        let mut citations = Vec::new();
        for step in steps {
            visit(&step.details, &mut citations);
        }
        citations
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_citations_from_json_encoded_tool_results() {
        let steps = vec![AgenticStepEvidence {
            step_type: "tool_calls".into(),
            tool_name: Some("search_factory_manuals".into()),
            details: json!({
                "result": r#"{
                    "results": [{
                        "title": "Robot 17 Service Manual",
                        "source": "robot-17-service-manual.md",
                        "excerpt": "Keep the robot paused until inspection criteria are satisfied."
                    }]
                }"#
            }),
        }];

        let citations = AgenticRetrievalProvider::step_citations(&steps);

        assert_eq!(citations.len(), 1);
        assert_eq!(
            citations[0].source.as_deref(),
            Some("robot-17-service-manual.md")
        );
        assert_eq!(
            citations[0].title.as_deref(),
            Some("Robot 17 Service Manual")
        );
    }
}

#[async_trait]
impl InferenceProvider for AgenticRetrievalProvider {
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
                message: "Agentic Retrieval is disabled or missing endpoint, agent, or token"
                    .into(),
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
                    message: format!(
                        "Agentic Retrieval knowledge-base endpoint returned HTTP {}",
                        status.as_u16()
                    ),
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
                    "Agentic Retrieval health request timed out".into()
                } else {
                    "Agentic Retrieval endpoint is unreachable".into()
                },
            },
        }
    }

    async fn complete_chat(&self, request: &ChatRequest) -> ChatResponse {
        let started = Instant::now();
        let Some(agent_id) = self.settings.resolved_agent_id() else {
            return self.error_response(
                request,
                started,
                "unconfigured_agent",
                "Agentic Retrieval agent ID is not configured".into(),
                false,
            );
        };

        let thread = match self
            .post_json(
                "/threads",
                json!({
                    "title": format!("Factory incident {}", request.request_id),
                    "metadata": {
                        "request_id": request.request_id,
                        "scenario_id": request.scenario_id
                    }
                }),
            )
            .await
        {
            Ok(value) => value,
            Err(message) => {
                return self.error_response(
                    request,
                    started,
                    "thread_creation_failed",
                    message,
                    true,
                );
            }
        };
        let Some(thread_id) = thread.get("id").and_then(Value::as_str) else {
            return self.error_response(
                request,
                started,
                "invalid_agentic_response",
                "Thread response did not contain an ID".into(),
                false,
            );
        };

        for message in &request.messages {
            if let Err(error) = self
                .post_json(
                    &format!("/threads/{thread_id}/messages"),
                    json!({
                        "role": message.role,
                        "content": message.content,
                        "metadata": { "request_id": request.request_id }
                    }),
                )
                .await
            {
                return self.error_response(
                    request,
                    started,
                    "message_creation_failed",
                    error,
                    true,
                );
            }
        }

        let run = match self
            .post_json(
                &format!("/threads/{thread_id}/runs?stream=false"),
                json!({
                    "agent_id": agent_id,
                    "instructions": "Use the configured factory knowledge sources and tools. Ground conclusions in retrieved evidence, identify uncertainty, and return advisory guidance suitable for operator review.",
                    "metadata": {
                        "request_id": request.request_id,
                        "scenario_id": request.scenario_id
                    }
                }),
            )
            .await
        {
            Ok(value) => value,
            Err(message) => {
                return self.error_response(
                    request,
                    started,
                    "run_creation_failed",
                    message,
                    true,
                );
            }
        };
        let Some(run_id) = run.get("id").and_then(Value::as_str) else {
            return self.error_response(
                request,
                started,
                "invalid_agentic_response",
                "Run response did not contain an ID".into(),
                false,
            );
        };

        let completed_run = loop {
            if started.elapsed() >= self.timeout {
                return self.error_response(
                    request,
                    started,
                    "agentic_timeout",
                    "Agentic Retrieval run timed out".into(),
                    true,
                );
            }
            let current = match self
                .get_json(&format!("/threads/{thread_id}/runs/{run_id}"))
                .await
            {
                Ok(value) => value,
                Err(message) => {
                    return self.error_response(
                        request,
                        started,
                        "run_status_failed",
                        message,
                        true,
                    );
                }
            };
            match current.get("status").and_then(Value::as_str) {
                Some("completed") => break current,
                Some("failed" | "cancelled" | "expired") => {
                    let status = current
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or("failed");
                    let detail = current
                        .get("last_error")
                        .map(Value::to_string)
                        .unwrap_or_else(|| "no error detail".into());
                    return self.error_response(
                        request,
                        started,
                        "agentic_run_failed",
                        format!("Agentic Retrieval run {status}: {detail}"),
                        false,
                    );
                }
                _ => {
                    sleep(Duration::from_millis(
                        self.settings.agent_poll_interval_ms.max(100),
                    ))
                    .await;
                }
            }
        };

        let messages = match self
            .get_json(&format!(
                "/threads/{thread_id}/messages?limit=100&order=asc"
            ))
            .await
        {
            Ok(value) => value,
            Err(message) => {
                return self.error_response(request, started, "message_read_failed", message, true);
            }
        };
        let steps = match self
            .get_json(&format!(
                "/threads/{thread_id}/runs/{run_id}/steps?limit=100&order=asc"
            ))
            .await
        {
            Ok(value) => value,
            Err(message) => {
                return self.error_response(request, started, "run_steps_failed", message, true);
            }
        };
        let Some(content) = Self::assistant_content(&messages) else {
            return self.error_response(
                request,
                started,
                "missing_agent_response",
                "Agentic Retrieval run completed without an assistant message".into(),
                false,
            );
        };
        let status = completed_run
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("completed")
            .to_owned();
        let parsed_steps = Self::steps(&steps);
        let mut citations = Self::citations(&messages);
        citations.extend(Self::step_citations(&parsed_steps));
        citations.sort_by(|left, right| {
            (&left.source, &left.title, &left.excerpt).cmp(&(
                &right.source,
                &right.title,
                &right.excerpt,
            ))
        });
        citations.dedup_by(|left, right| {
            left.source == right.source
                && left.title == right.title
                && left.excerpt == right.excerpt
        });

        ChatResponse {
            request_id: request.request_id.clone(),
            provider_id: self.id.clone(),
            target_type: self.settings.target_type,
            model_id: agent_id.clone(),
            endpoint_alias: self.settings.endpoint_alias.clone(),
            content,
            finish_reason: Some("stop".into()),
            input_tokens: completed_run
                .pointer("/usage/prompt_tokens")
                .and_then(Value::as_u64),
            output_tokens: completed_run
                .pointer("/usage/completion_tokens")
                .and_then(Value::as_u64),
            latency_ms: started.elapsed().as_millis().max(1) as u64,
            fallback_used: false,
            success: true,
            timestamp: Utc::now(),
            confidence: None,
            raw_response_logged: false,
            mock: false,
            agentic_evidence: Some(AgenticRunEvidence {
                thread_id: thread_id.into(),
                run_id: run_id.into(),
                agent_id,
                status,
                steps: parsed_steps,
                citations,
            }),
            error: None,
        }
    }
}
