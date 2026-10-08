use crate::{
    domain::{
        ActionProposal, AdvisoryRequest, AdvisoryResponse, AdvisoryResponseStatus, Message,
        ProposedAction, RequestedTarget, RiskLevel,
    },
    services::{FastSlowCoordinator, IncidentStore, ScenarioService},
};
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS, Transport};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    env, fs,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::{MissedTickBehavior, interval, sleep};
use tracing::{info, warn};
use uuid::Uuid;

const ADVISORY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdvisoryMode {
    Direct,
    Mqtt,
}

#[derive(Debug, Clone)]
pub struct AdvisorySettings {
    pub mode: AdvisoryMode,
    pub factory_id: String,
    pub edge_id: String,
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub username: Option<String>,
    pub password: Option<String>,
    pub request_topic: String,
    pub response_topic: String,
    pub client_id: String,
    pub publish_interval: Duration,
    pub retry_interval: Duration,
}

impl AdvisorySettings {
    pub fn from_env() -> Result<Self> {
        let mode = match env::var("ADVISORY_TRANSPORT")
            .unwrap_or_else(|_| "direct".into())
            .to_ascii_lowercase()
            .as_str()
        {
            "direct" => AdvisoryMode::Direct,
            "mqtt" => AdvisoryMode::Mqtt,
            other => return Err(anyhow!("unsupported ADVISORY_TRANSPORT '{other}'")),
        };
        let factory_id = env::var("FACTORY_ID").unwrap_or_else(|_| "factory-demo-01".to_string());
        let edge_id = env::var("EDGE_ID").unwrap_or_else(|_| "edge-demo-01".to_string());
        let host = env::var("MQTT_HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let port = env_value("MQTT_PORT", 1883)?;
        let tls = env_bool("MQTT_TLS", false)?;
        let request_topic = env::var("MQTT_REQUEST_TOPIC")
            .unwrap_or_else(|_| format!("factory/v1/{factory_id}/advisory/requests"));
        let response_topic = env::var("MQTT_RESPONSE_TOPIC").unwrap_or_else(|_| {
            format!("factory/v1/{factory_id}/edges/{edge_id}/advisory/responses")
        });
        Ok(Self {
            mode,
            factory_id,
            edge_id: edge_id.clone(),
            host,
            port,
            tls,
            username: non_empty_env("MQTT_USERNAME"),
            password: non_empty_env("MQTT_PASSWORD"),
            request_topic,
            response_topic,
            client_id: env::var("MQTT_CLIENT_ID")
                .unwrap_or_else(|_| format!("governed-floor-{edge_id}")),
            publish_interval: Duration::from_secs(env_value("MQTT_PUBLISH_INTERVAL_SECONDS", 5)?),
            retry_interval: Duration::from_secs(env_value("MQTT_RETRY_INTERVAL_SECONDS", 30)?),
        })
    }

    pub fn worker_from_env() -> Result<Self> {
        let mut settings = Self::from_env()?;
        settings.mode = AdvisoryMode::Mqtt;
        settings.client_id =
            env::var("MQTT_CLIENT_ID").unwrap_or_else(|_| "factory-advisory-worker".into());
        Ok(settings)
    }

    pub fn mqtt_options(&self) -> MqttOptions {
        let mut options = MqttOptions::new(&self.client_id, &self.host, self.port);
        options
            .set_keep_alive(Duration::from_secs(20))
            .set_clean_session(false);
        if let Some(username) = &self.username {
            options.set_credentials(username, self.password.as_deref().unwrap_or_default());
        }
        if self.tls {
            options.set_transport(Transport::tls_with_default_config());
        }
        options
    }
}

#[derive(Clone)]
pub struct AdvisoryProcessor {
    scenarios: ScenarioService,
    coordinator: FastSlowCoordinator,
}

impl AdvisoryProcessor {
    pub fn new(scenarios: ScenarioService, coordinator: FastSlowCoordinator) -> Self {
        Self {
            scenarios,
            coordinator,
        }
    }

    pub async fn process(&self, request: &AdvisoryRequest) -> AdvisoryResponse {
        match self.process_inner(request).await {
            Ok(response) => response,
            Err(error) => AdvisoryResponse {
                schema_version: ADVISORY_SCHEMA_VERSION,
                message_id: response_message_id(request),
                request_message_id: request.message_id.clone(),
                incident_id: request.incident_id.clone(),
                evidence_version: request.evidence_version.clone(),
                status: AdvisoryResponseStatus::Failed,
                summary: "The factory advisory worker could not complete grounded analysis.".into(),
                proposal: None,
                completed_at: Utc::now(),
                error: Some(error.to_string()),
            },
        }
    }

    async fn process_inner(&self, request: &AdvisoryRequest) -> Result<AdvisoryResponse> {
        if Utc::now() > request.expires_at {
            return Err(anyhow!("advisory request expired before processing"));
        }
        let factory_scenario = self.scenarios.get("cross-machine-correlation")?;
        let package = serde_json::json!({
            "incident_id": request.incident_id,
            "machine_id": request.machine_id,
            "factory_id": request.factory_id,
            "edge_id": request.edge_id,
            "machine_model": request.machine_model,
            "manual_revision": request.manual_revision,
            "alarm": request.alarm,
            "signals": request.signals,
            "evidence_version": request.evidence_version,
            "occurred_at": request.occurred_at
        });
        let factory_run = self
            .coordinator
            .run(
                &factory_scenario,
                RequestedTarget::Edge,
                vec![
                    Message {
                        role: "user".into(),
                        content: format!(
                            "Retrieve the applicable machine manual guidance and analyze this alarm using only grounded evidence. Return a concise operator-facing advisory in at most 160 words. Explain the likely condition, the immediate inspection priority, and whether a bounded 15% speed reduction is a reasonable temporary action. Do not claim that an action has executed. Do not narrate planning or tool calls. Do not emit [cite:N] markers because citations are rendered separately.\nIncident evidence:\n{}",
                            serde_json::to_string_pretty(&package)?
                        ),
                    },
                    Message {
                        role: "assistant".into(),
                        content: format!(
                            "Machine-local observation: {}",
                            request.local_observation
                        ),
                    },
                ],
                Some(false),
            )
            .await?;
        let slow = factory_run
            .slow_response
            .ok_or_else(|| anyhow!("factory advisory returned no response"))?;
        if !slow.success {
            return Err(anyhow!(
                "factory advisory failed: {}",
                slow.error
                    .map(|provider_error| provider_error.message)
                    .unwrap_or_else(|| "unknown provider error".into())
            ));
        }
        let sources = slow
            .agentic_evidence
            .as_ref()
            .map(|evidence| evidence.citations.clone())
            .unwrap_or_default();
        let grounded = !sources.is_empty();
        let summary = operator_advisory_summary(&slow.content).unwrap_or_else(|| {
            format!(
                "The factory advisory worker retrieved {} applicable manual source{}. Verify the vibration and temperature readings, inspect bearing mounting and lubrication, and keep the machine under observation. A temporary 15% speed reduction is available for operator review; the Edge Guard will recheck current machine evidence before execution.",
                sources.len(),
                if sources.len() == 1 { "" } else { "s" }
            )
        });
        let proposal = ActionProposal {
            proposal_id: Uuid::new_v4().to_string(),
            incident_id: request.incident_id.clone(),
            root_cause_hypothesis: format!(
                "Grounded bearing-vibration advisory for {}.",
                request.machine_id
            ),
            operator_summary: summary.clone(),
            reasoning: summary.clone(),
            proposed_action: ProposedAction {
                action_id: "reduce_speed".into(),
                parameters: serde_json::json!({"reduction_percent": 15}),
                risk_level: RiskLevel::Medium,
            },
            sources,
            alternatives: vec![ProposedAction {
                action_id: "request_inspection".into(),
                parameters: serde_json::json!({"priority": "prompt"}),
                risk_level: RiskLevel::Low,
            }],
            created_at: Utc::now(),
        };
        Ok(AdvisoryResponse {
            schema_version: ADVISORY_SCHEMA_VERSION,
            message_id: response_message_id(request),
            request_message_id: request.message_id.clone(),
            incident_id: request.incident_id.clone(),
            evidence_version: request.evidence_version.clone(),
            status: if grounded {
                AdvisoryResponseStatus::Grounded
            } else {
                AdvisoryResponseStatus::Ungrounded
            },
            summary,
            proposal: Some(proposal),
            completed_at: Utc::now(),
            error: None,
        })
    }
}

#[derive(Clone)]
pub struct MqttAdvisoryMachine {
    store: IncidentStore,
    settings: AdvisorySettings,
}

impl MqttAdvisoryMachine {
    pub fn new(store: IncidentStore, settings: AdvisorySettings) -> Self {
        Self { store, settings }
    }

    pub async fn run(self) {
        loop {
            if let Err(error) = self.run_session().await {
                warn!(event = "mqtt_machine_session_failed", error = %error);
                sleep(Duration::from_secs(5)).await;
            }
        }
    }

    async fn run_session(&self) -> Result<()> {
        let (client, mut eventloop) = AsyncClient::new(self.settings.mqtt_options(), 100);
        client
            .subscribe(&self.settings.response_topic, QoS::AtLeastOnce)
            .await?;
        info!(
            event = "mqtt_machine_connected",
            request_topic = self.settings.request_topic,
            response_topic = self.settings.response_topic
        );
        let mut publish_tick = interval(self.settings.publish_interval);
        publish_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = publish_tick.tick() => {
                    self.publish_pending(&client).await?;
                }
                event = eventloop.poll() => {
                    if let Event::Incoming(Incoming::Publish(message)) = event? {
                        let response: AdvisoryResponse = serde_json::from_slice(&message.payload)
                            .context("parse MQTT advisory response")?;
                        self.store.apply_advisory_response(&response)?;
                        info!(
                            event = "mqtt_advisory_response_applied",
                            incident_id = response.incident_id,
                            message_id = response.message_id
                        );
                    }
                }
            }
        }
    }

    async fn publish_pending(&self, client: &AsyncClient) -> Result<()> {
        for pending in self.store.pending_advisory_requests(20)? {
            let payload = serde_json::to_vec(&pending.payload)?;
            match client
                .publish(&pending.topic, QoS::AtLeastOnce, false, payload)
                .await
            {
                Ok(()) => {
                    self.store.record_advisory_publish_attempt(
                        &pending.message_id,
                        self.settings.retry_interval,
                        None,
                    )?;
                    info!(
                        event = "mqtt_advisory_request_published",
                        incident_id = pending.payload.incident_id,
                        message_id = pending.message_id,
                        attempt = pending.attempt_count + 1
                    );
                }
                Err(error) => {
                    self.store.record_advisory_publish_attempt(
                        &pending.message_id,
                        Duration::from_secs(5),
                        Some(&error.to_string()),
                    )?;
                    return Err(error.into());
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct WorkerJobStore {
    connection: Arc<Mutex<Connection>>,
}

enum JobClaim {
    Acquired,
    Processing,
    Completed(Box<AdvisoryResponse>),
}

impl WorkerJobStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS advisory_jobs (
                request_message_id TEXT PRIMARY KEY,
                incident_id TEXT NOT NULL,
                request_json TEXT NOT NULL,
                status TEXT NOT NULL,
                response_json TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                last_error TEXT
            );
            ",
        )?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    fn claim(&self, request: &AdvisoryRequest) -> Result<JobClaim> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow!("advisory worker database lock is poisoned"))?;
        let existing = connection
            .query_row(
                "
                SELECT status, response_json
                FROM advisory_jobs
                WHERE request_message_id = ?1
                ",
                [&request.message_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        if let Some((status, response_json)) = existing {
            if status == "completed" {
                let response = response_json
                    .ok_or_else(|| anyhow!("completed advisory job has no response"))
                    .and_then(|value| {
                        serde_json::from_str(&value).context("parse stored advisory response")
                    })?;
                return Ok(JobClaim::Completed(Box::new(response)));
            }
            if status == "processing" {
                return Ok(JobClaim::Processing);
            }
        }
        connection.execute(
            "
            INSERT INTO advisory_jobs (
                request_message_id, incident_id, request_json, status, created_at, updated_at
            ) VALUES (?1, ?2, ?3, 'processing', ?4, ?4)
            ON CONFLICT(request_message_id) DO UPDATE SET
                status = CASE
                    WHEN advisory_jobs.status = 'completed' THEN advisory_jobs.status
                    ELSE 'processing'
                END,
                updated_at = excluded.updated_at
            ",
            params![
                request.message_id,
                request.incident_id,
                serde_json::to_string(request)?,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(JobClaim::Acquired)
    }

    pub fn complete(&self, response: &AdvisoryResponse) -> Result<()> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow!("advisory worker database lock is poisoned"))?;
        connection.execute(
            "
            UPDATE advisory_jobs
            SET status = 'completed',
                response_json = ?2,
                updated_at = ?3,
                last_error = ?4
            WHERE request_message_id = ?1
            ",
            params![
                response.request_message_id,
                serde_json::to_string(response)?,
                Utc::now().to_rfc3339(),
                response.error,
            ],
        )?;
        Ok(())
    }
}

pub struct MqttAdvisoryWorker {
    processor: AdvisoryProcessor,
    settings: AdvisorySettings,
    jobs: WorkerJobStore,
}

impl MqttAdvisoryWorker {
    pub fn new(
        processor: AdvisoryProcessor,
        settings: AdvisorySettings,
        jobs: WorkerJobStore,
    ) -> Self {
        Self {
            processor,
            settings,
            jobs,
        }
    }

    pub async fn run(self) -> Result<()> {
        loop {
            if let Err(error) = self.run_session().await {
                warn!(event = "mqtt_worker_session_failed", error = %error);
                sleep(Duration::from_secs(5)).await;
            }
        }
    }

    async fn run_session(&self) -> Result<()> {
        let (client, mut eventloop) = AsyncClient::new(self.settings.mqtt_options(), 100);
        client
            .subscribe(&self.settings.request_topic, QoS::AtLeastOnce)
            .await?;
        info!(
            event = "mqtt_worker_connected",
            request_topic = self.settings.request_topic
        );
        loop {
            if let Event::Incoming(Incoming::Publish(message)) = eventloop.poll().await? {
                let request: AdvisoryRequest = serde_json::from_slice(&message.payload)
                    .context("parse MQTT advisory request")?;
                match self.jobs.claim(&request)? {
                    JobClaim::Processing => {}
                    JobClaim::Completed(response) => {
                        client
                            .publish(
                                &request.response_topic,
                                QoS::AtLeastOnce,
                                false,
                                serde_json::to_vec(response.as_ref())?,
                            )
                            .await?;
                    }
                    JobClaim::Acquired => {
                        let processor = self.processor.clone();
                        let jobs = self.jobs.clone();
                        let publisher = client.clone();
                        tokio::spawn(async move {
                            let response = processor.process(&request).await;
                            if let Err(error) = jobs.complete(&response) {
                                warn!(
                                    event = "mqtt_advisory_job_persist_failed",
                                    request_message_id = request.message_id,
                                    error = %error
                                );
                                return;
                            }
                            match serde_json::to_vec(&response) {
                                Ok(payload) => {
                                    if let Err(error) = publisher
                                        .publish(
                                            &request.response_topic,
                                            QoS::AtLeastOnce,
                                            false,
                                            payload,
                                        )
                                        .await
                                    {
                                        warn!(
                                            event = "mqtt_advisory_response_publish_failed",
                                            request_message_id = request.message_id,
                                            error = %error
                                        );
                                    } else {
                                        info!(
                                            event = "mqtt_advisory_response_published",
                                            incident_id = request.incident_id,
                                            request_message_id = request.message_id,
                                            response_message_id = response.message_id
                                        );
                                    }
                                }
                                Err(error) => warn!(
                                    event = "mqtt_advisory_response_serialize_failed",
                                    request_message_id = request.message_id,
                                    error = %error
                                ),
                            }
                        });
                    }
                }
            }
        }
    }
}

pub fn build_advisory_request(
    record: &crate::domain::IncidentRecord,
    settings: &AdvisorySettings,
) -> AdvisoryRequest {
    let evidence_version = record.incident.timestamp.to_rfc3339();
    AdvisoryRequest {
        schema_version: ADVISORY_SCHEMA_VERSION,
        message_id: format!(
            "advisory-request-{}-{}",
            record.incident.incident_id,
            Uuid::new_v4()
        ),
        incident_id: record.incident.incident_id.clone(),
        factory_id: settings.factory_id.clone(),
        edge_id: settings.edge_id.clone(),
        machine_id: record.incident.machine_id.clone(),
        machine_model: record.incident.machine_model.clone(),
        manual_revision: record.incident.manual_revision.clone(),
        evidence_version,
        occurred_at: record.incident.timestamp,
        expires_at: Utc::now() + chrono::Duration::minutes(10),
        alarm: record.incident.alarm.clone(),
        signals: record
            .incident
            .context
            .get("signals")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        local_observation: record
            .incident
            .local_assessment
            .as_ref()
            .map(|assessment| assessment.summary.clone())
            .unwrap_or_else(|| "No local observation was available.".into()),
        response_topic: settings.response_topic.clone(),
    }
}

fn response_message_id(request: &AdvisoryRequest) -> String {
    format!(
        "advisory-response-{}-{}",
        request.incident_id, request.evidence_version
    )
}

fn operator_advisory_summary(content: &str) -> Option<String> {
    let without_followup = content.split("<followup>").next().unwrap_or(content).trim();
    let advisory_start = [
        "Likely condition:",
        "Observed condition:",
        "Observed:",
        "Assessment:",
        "Factory advisory:",
    ]
    .iter()
    .filter_map(|marker| without_followup.find(marker))
    .min()
    .unwrap_or(0);
    let summary = without_followup[advisory_start..]
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with("I will ")
                && !line.starts_with("I'll ")
                && !line.starts_with("First, ")
        })
        .collect::<Vec<_>>()
        .join(" ");
    (!summary.is_empty()).then_some(summary)
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn env_bool(name: &str, default: bool) -> Result<bool> {
    match non_empty_env(name) {
        None => Ok(default),
        Some(value) => value
            .parse()
            .map_err(|_| anyhow!("{name} must be true or false")),
    }
}

fn env_value<T>(name: &str, default: T) -> Result<T>
where
    T: std::str::FromStr,
{
    match non_empty_env(name) {
        None => Ok(default),
        Some(value) => value
            .parse()
            .map_err(|_| anyhow!("{name} has an invalid value")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advisory_summary_removes_planning_and_followup_text() {
        let content = "I will list manuals.Likely condition: Inspect the bearing.\n<followup>\nShow another document.";
        assert_eq!(
            operator_advisory_summary(content),
            Some("Likely condition: Inspect the bearing.".into())
        );
    }
}
