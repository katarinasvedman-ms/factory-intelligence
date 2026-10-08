use crate::domain::{
    AuditEvent, FabricIncidentEvent, FabricOutboxEvent, FabricPublicationStatus, IncidentRecord,
};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use azure_identity::DeveloperToolsCredential;
use azure_messaging_eventhubs::{ProducerClient, models::EventData};
use serde_json::{Map, Value};
use std::{env, sync::Arc, time::Duration};
use tracing::{info, warn};

use super::IncidentStore;

const FABRIC_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct FabricSettings {
    pub enabled: bool,
    pub factory_id: String,
    pub eventhub_host: Option<String>,
    pub eventhub_name: Option<String>,
    pub batch_size: usize,
    pub publish_interval: Duration,
    pub max_attempts: u32,
}

impl FabricSettings {
    pub fn from_env() -> Result<Self> {
        let enabled = env_bool("FABRIC_EXPORT_ENABLED", false)?;
        let factory_id = env::var("FACTORY_ID").unwrap_or_else(|_| "factory-demo-01".to_string());
        let eventhub_host = non_empty_env("FABRIC_EVENTHUB_HOST");
        let eventhub_name = non_empty_env("FABRIC_EVENTHUB_NAME");
        let batch_size = env_usize("FABRIC_PUBLISH_BATCH_SIZE", 50)?;
        let publish_interval = Duration::from_secs(env_u64("FABRIC_PUBLISH_INTERVAL_SECONDS", 5)?);
        let max_attempts = env_u32("FABRIC_PUBLISH_MAX_ATTEMPTS", 10)?;

        if batch_size == 0 {
            return Err(anyhow!(
                "FABRIC_PUBLISH_BATCH_SIZE must be greater than zero"
            ));
        }
        if publish_interval.is_zero() {
            return Err(anyhow!(
                "FABRIC_PUBLISH_INTERVAL_SECONDS must be greater than zero"
            ));
        }
        if max_attempts == 0 {
            return Err(anyhow!(
                "FABRIC_PUBLISH_MAX_ATTEMPTS must be greater than zero"
            ));
        }
        if enabled && (eventhub_host.is_none() || eventhub_name.is_none()) {
            return Err(anyhow!(
                "FABRIC_EVENTHUB_HOST and FABRIC_EVENTHUB_NAME are required when FABRIC_EXPORT_ENABLED is true"
            ));
        }

        Ok(Self {
            enabled,
            factory_id,
            eventhub_host,
            eventhub_name,
            batch_size,
            publish_interval,
            max_attempts,
        })
    }
}

#[async_trait]
pub trait IncidentEventPublisher: Send + Sync {
    async fn publish(&self, events: &[FabricIncidentEvent]) -> Result<()>;
}

pub struct DisabledPublisher;

#[async_trait]
impl IncidentEventPublisher for DisabledPublisher {
    async fn publish(&self, _events: &[FabricIncidentEvent]) -> Result<()> {
        Ok(())
    }
}

pub struct EventHubPublisher {
    host: String,
    eventhub: String,
}

impl EventHubPublisher {
    pub fn new(settings: &FabricSettings) -> Result<Self> {
        let host = settings
            .eventhub_host
            .as_deref()
            .ok_or_else(|| anyhow!("Fabric Event Hubs host is not configured"))?;
        let eventhub = settings
            .eventhub_name
            .as_deref()
            .ok_or_else(|| anyhow!("Fabric Event Hubs name is not configured"))?;
        let host = host
            .trim()
            .trim_start_matches("sb://")
            .trim_end_matches('/')
            .to_string();
        Ok(Self {
            host,
            eventhub: eventhub.to_string(),
        })
    }
}

#[async_trait]
impl IncidentEventPublisher for EventHubPublisher {
    async fn publish(&self, events: &[FabricIncidentEvent]) -> Result<()> {
        let credential = DeveloperToolsCredential::new(None)?;
        let producer = ProducerClient::builder()
            .with_application_id("governed-floor".to_string())
            .open(&self.host, &self.eventhub, credential)
            .await?;
        let batch = producer.create_batch(None).await?;
        for event in events {
            let payload = serde_json::to_vec(event)?;
            let event_data = EventData::builder()
                .with_content_type("application/json".to_string())
                .with_body(payload)
                .add_property("event_type".to_string(), event.event_type.clone())
                .add_property("factory_id".to_string(), event.factory_id.clone())
                .build();
            if !batch.try_add_event_data(event_data, None)? {
                return Err(anyhow!(
                    "Fabric publication batch exceeded the Event Hubs message size limit"
                ));
            }
        }
        producer.send_batch(batch, None).await?;
        producer.close().await?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct FabricPublicationService {
    store: IncidentStore,
    settings: FabricSettings,
    publisher: Arc<dyn IncidentEventPublisher>,
}

impl FabricPublicationService {
    pub fn new(
        store: IncidentStore,
        settings: FabricSettings,
        publisher: Arc<dyn IncidentEventPublisher>,
    ) -> Self {
        Self {
            store,
            settings,
            publisher,
        }
    }

    pub fn settings(&self) -> &FabricSettings {
        &self.settings
    }

    pub fn status(&self) -> Result<FabricPublicationStatus> {
        self.store
            .fabric_publication_status(self.settings.enabled, &self.settings.factory_id)
    }

    pub async fn publish_once(&self) -> Result<usize> {
        if !self.settings.enabled {
            return Ok(0);
        }

        let events = self.store.pending_fabric_events(self.settings.batch_size)?;
        if events.is_empty() {
            return Ok(0);
        }

        let payloads = events
            .iter()
            .map(|event| event.payload.clone())
            .collect::<Vec<_>>();
        match self.publisher.publish(&payloads).await {
            Ok(()) => {
                self.store.mark_fabric_published(
                    &events
                        .iter()
                        .map(|event| event.event_id.as_str())
                        .collect::<Vec<_>>(),
                )?;
                Ok(events.len())
            }
            Err(error) => {
                self.record_failure(&events, &error.to_string())?;
                Err(error)
            }
        }
    }

    pub async fn run(self) {
        if !self.settings.enabled {
            return;
        }
        info!(
            event = "fabric_publisher_started",
            factory_id = self.settings.factory_id
        );
        loop {
            match self.publish_once().await {
                Ok(published) if published > 0 => {
                    info!(event = "fabric_events_published", event_count = published);
                }
                Ok(_) => {}
                Err(error) => {
                    warn!(
                        event = "fabric_publication_failed",
                        error = %error
                    );
                }
            }
            tokio::time::sleep(self.settings.publish_interval).await;
        }
    }

    fn record_failure(&self, events: &[FabricOutboxEvent], error: &str) -> Result<()> {
        let error = truncate(error, 500);
        for event in events {
            let next_attempt = event.attempt_count.saturating_add(1);
            let terminal = next_attempt >= self.settings.max_attempts;
            let delay_seconds = 2_u64.saturating_pow(next_attempt.min(8)).min(300);
            self.store.record_fabric_failure(
                &event.event_id,
                &error,
                Duration::from_secs(delay_seconds),
                terminal,
            )?;
        }
        Ok(())
    }
}

pub fn project_fabric_event(
    record: &IncidentRecord,
    event: &AuditEvent,
    factory_id: &str,
) -> FabricIncidentEvent {
    let details = sanitize_details(&event.details);
    let correlation_id = details
        .get("related_incident_id")
        .or_else(|| details.get("upstream_incident_id"))
        .and_then(Value::as_str)
        .map(str::to_string);

    FabricIncidentEvent {
        schema_version: FABRIC_SCHEMA_VERSION,
        event_id: event.event_id.clone(),
        occurred_at: event.timestamp,
        factory_id: factory_id.to_string(),
        line_id: record.incident.line_id.clone(),
        machine_id: record.incident.machine_id.clone(),
        incident_id: record.incident.incident_id.clone(),
        event_type: event.event_type.clone(),
        incident_status: record.status,
        severity: record.incident.alarm.severity.clone(),
        summary: event.summary.clone(),
        correlation_id,
        details,
    }
}

fn sanitize_details(details: &Value) -> Value {
    let Some(details) = details.as_object() else {
        return Value::Object(Map::new());
    };
    let mut sanitized = Map::new();
    for key in [
        "vendor_profile",
        "escalated",
        "related_incident_id",
        "proposal_id",
        "source_count",
        "upstream_incident_id",
        "decision_id",
        "decision_source",
        "executed",
        "outcome",
        "expected_response",
        "requested_reduction_percent",
        "connectivity",
        "factory_agent_used",
        "queued_for_sync",
    ] {
        if let Some(value) = details.get(key) {
            sanitized.insert(key.to_string(), value.clone());
        }
    }
    Value::Object(sanitized)
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn env_bool(name: &str, default: bool) -> Result<bool> {
    match non_empty_env(name) {
        None => Ok(default),
        Some(value) => value
            .parse::<bool>()
            .map_err(|_| anyhow!("{name} must be true or false")),
    }
}

fn env_u64(name: &str, default: u64) -> Result<u64> {
    match non_empty_env(name) {
        None => Ok(default),
        Some(value) => value
            .parse::<u64>()
            .map_err(|_| anyhow!("{name} must be a positive integer")),
    }
}

fn env_u32(name: &str, default: u32) -> Result<u32> {
    let value = env_u64(name, u64::from(default))?;
    u32::try_from(value).map_err(|_| anyhow!("{name} is too large"))
}

fn env_usize(name: &str, default: usize) -> Result<usize> {
    let value = env_u64(name, default as u64)?;
    usize::try_from(value).map_err(|_| anyhow!("{name} is too large"))
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ConnectivityState, IncidentPackage, IncidentStatus, NormalizedAlarm};
    use chrono::Utc;
    use serde_json::json;
    use std::sync::Mutex;

    fn test_record() -> IncidentRecord {
        let now = Utc::now();
        IncidentRecord {
            incident: IncidentPackage {
                incident_id: "incident-1".into(),
                machine_id: "press-04".into(),
                line_id: "line-04".into(),
                vendor_profile: "northstar".into(),
                machine_model: "press".into(),
                firmware_version: "1".into(),
                manual_revision: "1".into(),
                timestamp: now,
                alarm: NormalizedAlarm {
                    code: "drive_trip".into(),
                    raw_code: "NS-1".into(),
                    raw_text: "secret raw text".into(),
                    severity: "critical".into(),
                },
                context: json!({}),
                local_assessment: None,
                connectivity: ConnectivityState::Connected,
            },
            status: IncidentStatus::Executed,
            proposal: None,
            guard_decision: None,
            audit: vec![],
            created_at: now,
            updated_at: now,
        }
    }

    fn test_event() -> AuditEvent {
        let now = Utc::now();
        AuditEvent {
            event_id: "event-1".into(),
            incident_id: "incident-1".into(),
            event_type: "guard_executed".into(),
            summary: "Approved action executed.".into(),
            timestamp: now,
            details: json!({
                "decision_id": "decision-1",
                "approved_by": "operator@example.com",
                "raw_text": "do not export",
                "executed": true
            }),
        }
    }

    #[test]
    fn publication_projection_removes_sensitive_audit_details() {
        let record = test_record();
        let event = test_event();

        let projected = project_fabric_event(&record, &event, "factory-01");

        assert_eq!(projected.factory_id, "factory-01");
        assert_eq!(projected.details["decision_id"], "decision-1");
        assert_eq!(projected.details["executed"], true);
        assert!(projected.details.get("approved_by").is_none());
        assert!(projected.details.get("raw_text").is_none());
    }

    struct RecordingPublisher {
        batches: Mutex<Vec<Vec<FabricIncidentEvent>>>,
    }

    #[async_trait]
    impl IncidentEventPublisher for RecordingPublisher {
        async fn publish(&self, events: &[FabricIncidentEvent]) -> Result<()> {
            self.batches.lock().unwrap().push(events.to_vec());
            Ok(())
        }
    }

    #[tokio::test]
    async fn successful_publication_marks_transactional_outbox_events_published() {
        let store = IncidentStore::in_memory().unwrap();
        let mut record = test_record();
        record.audit.push(test_event());
        store.save(&record).unwrap();
        let publisher = Arc::new(RecordingPublisher {
            batches: Mutex::new(vec![]),
        });
        let service = FabricPublicationService::new(
            store,
            FabricSettings {
                enabled: true,
                factory_id: "factory-test".into(),
                eventhub_host: Some("unused.example".into()),
                eventhub_name: Some("unused".into()),
                batch_size: 50,
                publish_interval: Duration::from_secs(5),
                max_attempts: 3,
            },
            publisher.clone(),
        );

        assert_eq!(service.publish_once().await.unwrap(), 1);
        assert_eq!(publisher.batches.lock().unwrap().len(), 1);
        let status = service.status().unwrap();
        assert_eq!(status.pending, 0);
        assert_eq!(status.published, 1);
        assert_eq!(status.failed, 0);
    }

    struct FailingPublisher;

    #[async_trait]
    impl IncidentEventPublisher for FailingPublisher {
        async fn publish(&self, _events: &[FabricIncidentEvent]) -> Result<()> {
            Err(anyhow!("Fabric unavailable"))
        }
    }

    #[tokio::test]
    async fn publication_failure_preserves_local_incident_and_records_failure() {
        let store = IncidentStore::in_memory().unwrap();
        let mut record = test_record();
        record.audit.push(test_event());
        store.save(&record).unwrap();
        let service = FabricPublicationService::new(
            store.clone(),
            FabricSettings {
                enabled: true,
                factory_id: "factory-test".into(),
                eventhub_host: Some("unused.example".into()),
                eventhub_name: Some("unused".into()),
                batch_size: 50,
                publish_interval: Duration::from_secs(5),
                max_attempts: 1,
            },
            Arc::new(FailingPublisher),
        );

        assert!(service.publish_once().await.is_err());
        assert_eq!(
            store.get("incident-1").unwrap().incident.machine_id,
            "press-04"
        );
        let status = service.status().unwrap();
        assert_eq!(status.pending, 0);
        assert_eq!(status.published, 0);
        assert_eq!(status.failed, 1);
        assert_eq!(status.last_error.as_deref(), Some("Fabric unavailable"));
    }
}
