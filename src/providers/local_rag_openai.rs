use super::{InferenceProvider, openai_compatible::OpenAiCompatibleProvider};
use crate::{
    config::ProviderSettings,
    domain::{
        AgenticRunEvidence, AgenticStepEvidence, ChatRequest, ChatResponse, KnowledgeCitation,
        Message, ProviderCapabilities, ProviderStatus, TargetType,
    },
};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde_json::json;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Clone)]
struct KnowledgeDocument {
    title: String,
    source: String,
    content: String,
}

pub struct LocalRagOpenAiProvider {
    inner: OpenAiCompatibleProvider,
    documents: Vec<KnowledgeDocument>,
}

impl LocalRagOpenAiProvider {
    pub fn new(id: &str, settings: ProviderSettings, timeout_seconds: f64) -> Result<Self> {
        let knowledge_path = settings
            .knowledge_path
            .as_deref()
            .context("local_rag_openai provider requires knowledge_path")?;
        let documents = load_documents(Path::new(knowledge_path))?;
        if documents.is_empty() {
            bail!("knowledge_path contains no Markdown documents");
        }
        Ok(Self {
            inner: OpenAiCompatibleProvider::new(id, settings, timeout_seconds)?,
            documents,
        })
    }

    fn retrieve(&self, request: &ChatRequest) -> Vec<&KnowledgeDocument> {
        let query = request
            .messages
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let terms = tokenize(&query);
        let mut ranked = self
            .documents
            .iter()
            .map(|document| {
                let haystack = format!("{} {}", document.title, document.content).to_lowercase();
                let score = terms
                    .iter()
                    .map(|term| haystack.matches(term).count())
                    .sum::<usize>();
                (score, document)
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.title.cmp(&right.1.title))
        });
        let relevant = ranked
            .iter()
            .filter(|(score, _)| *score > 0)
            .take(3)
            .map(|(_, document)| *document)
            .collect::<Vec<_>>();
        if relevant.is_empty() {
            ranked
                .into_iter()
                .take(2)
                .map(|(_, document)| document)
                .collect()
        } else {
            relevant
        }
    }
}

#[async_trait]
impl InferenceProvider for LocalRagOpenAiProvider {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn target_type(&self) -> TargetType {
        self.inner.target_type()
    }

    fn is_mock(&self) -> bool {
        false
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.inner.capabilities()
    }

    async fn health(&self) -> ProviderStatus {
        self.inner.health().await
    }

    async fn complete_chat(&self, request: &ChatRequest) -> ChatResponse {
        let retrieved = self.retrieve(request);
        let context = retrieved
            .iter()
            .enumerate()
            .map(|(index, document)| {
                let content = document.content.chars().take(4_000).collect::<String>();
                format!(
                    "[Document {}: {}]\nSource: {}\n{}",
                    index + 1,
                    document.title,
                    document.source,
                    content
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let mut grounded_request = request.clone();
        grounded_request.messages.insert(
            0,
            Message {
                role: "system".into(),
                content: format!(
                    "Use only the supplied factory-maintenance documents as grounding evidence. \
                     Cite relevant documents by title, distinguish evidence from inference, and \
                     return advisory guidance for operator review. Do not authorize equipment \
                     actuation.\n\n{context}"
                ),
            },
        );

        let mut response = self.inner.complete_chat(&grounded_request).await;
        if response.success {
            let citations = retrieved
                .iter()
                .map(|document| KnowledgeCitation {
                    title: Some(document.title.clone()),
                    source: Some(document.source.clone()),
                    excerpt: Some(excerpt(&document.content)),
                })
                .collect::<Vec<_>>();
            response.agentic_evidence = Some(AgenticRunEvidence {
                thread_id: format!("local-rag-{}", request.request_id),
                run_id: Uuid::new_v4().to_string(),
                agent_id: "application-local-rag".into(),
                status: "completed".into(),
                steps: vec![AgenticStepEvidence {
                    step_type: "local_retrieval".into(),
                    tool_name: Some("local_document_search".into()),
                    details: json!({
                        "retrieved_documents": citations.len(),
                        "knowledge_source": "data/knowledge/source",
                        "retrieval": "application-side lexical ranking"
                    }),
                }],
                citations,
            });
        }
        response
    }
}

fn load_documents(path: &Path) -> Result<Vec<KnowledgeDocument>> {
    let mut paths = fs::read_dir(path)
        .with_context(|| format!("failed to read knowledge directory {}", path.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .collect::<Vec<PathBuf>>();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let title = content
                .lines()
                .find_map(|line| line.strip_prefix("# "))
                .unwrap_or_else(|| {
                    path.file_stem()
                        .and_then(|name| name.to_str())
                        .unwrap_or("Factory document")
                })
                .to_string();
            Ok(KnowledgeDocument {
                title,
                source: path.to_string_lossy().replace('\\', "/"),
                content,
            })
        })
        .collect()
}

fn tokenize(value: &str) -> HashSet<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|term| term.len() >= 4)
        .collect()
}

fn excerpt(content: &str) -> String {
    content
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .flat_map(str::split_whitespace)
        .take(55)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenization_ignores_short_terms() {
        let terms = tokenize("Robot 17 has a hot bearing");
        assert!(terms.contains("robot"));
        assert!(terms.contains("bearing"));
        assert!(!terms.contains("has"));
    }

    #[test]
    fn excerpt_skips_heading() {
        assert_eq!(
            excerpt("# Manual\nInspect the bearing first."),
            "Inspect the bearing first."
        );
    }
}
