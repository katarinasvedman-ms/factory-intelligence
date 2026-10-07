use crate::{
    domain::{FabricOutboxEvent, FabricPublicationStatus, IncidentRecord},
    services::fabric::project_fabric_event,
};
use anyhow::{Context, Result, anyhow};
use chrono::{Duration as ChronoDuration, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone)]
pub struct IncidentStore {
    connection: Arc<Mutex<Connection>>,
    factory_id: Arc<str>,
}

impl IncidentStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let factory_id = env::var("FACTORY_ID").unwrap_or_else(|_| "factory-demo-01".to_string());
        Self::open_for_factory(path, factory_id)
    }

    pub fn open_for_factory(path: impl AsRef<Path>, factory_id: impl Into<String>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).with_context(|| {
                format!("create incident database directory {}", parent.display())
            })?;
        }
        let connection = Connection::open(path)
            .with_context(|| format!("open incident database {}", path.display()))?;
        Self::from_connection(connection, factory_id.into())
    }

    pub fn open_default() -> Result<Self> {
        let path = std::env::var("FACTORY_DB_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("data/governed-floor.db"));
        Self::open(path)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?, "factory-test".into())
    }

    fn from_connection(connection: Connection, factory_id: String) -> Result<Self> {
        connection.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS incidents (
                incident_id TEXT PRIMARY KEY,
                machine_id TEXT NOT NULL,
                line_id TEXT NOT NULL,
                vendor_profile TEXT NOT NULL,
                status TEXT NOT NULL,
                record_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_incidents_updated_at
                ON incidents(updated_at DESC);
            CREATE TABLE IF NOT EXISTS app_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            INSERT OR IGNORE INTO app_state(key, value)
                VALUES ('factory_connected', 'true');
            CREATE TABLE IF NOT EXISTS fabric_outbox (
                event_id TEXT PRIMARY KEY,
                incident_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                attempt_count INTEGER NOT NULL DEFAULT 0,
                next_attempt_at TEXT,
                published_at TEXT,
                last_error TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_fabric_outbox_pending
                ON fabric_outbox(status, next_attempt_at, occurred_at);
            ",
        )?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            factory_id: factory_id.into(),
        })
    }

    pub fn save(&self, record: &IncidentRecord) -> Result<()> {
        let json = serde_json::to_string(record)?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| anyhow!("incident database lock is poisoned"))?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "
            INSERT INTO incidents (
                incident_id, machine_id, line_id, vendor_profile, status,
                record_json, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(incident_id) DO UPDATE SET
                machine_id = excluded.machine_id,
                line_id = excluded.line_id,
                vendor_profile = excluded.vendor_profile,
                status = excluded.status,
                record_json = excluded.record_json,
                updated_at = excluded.updated_at
            ",
            params![
                record.incident.incident_id,
                record.incident.machine_id,
                record.incident.line_id,
                record.incident.vendor_profile,
                serde_json::to_string(&record.status)?,
                json,
                record.created_at.to_rfc3339(),
                record.updated_at.to_rfc3339(),
            ],
        )?;
        for event in &record.audit {
            let payload = project_fabric_event(record, event, &self.factory_id);
            transaction.execute(
                "
                INSERT OR IGNORE INTO fabric_outbox (
                    event_id, incident_id, event_type, occurred_at, payload_json
                ) VALUES (?1, ?2, ?3, ?4, ?5)
                ",
                params![
                    payload.event_id,
                    payload.incident_id,
                    payload.event_type,
                    payload.occurred_at.to_rfc3339(),
                    serde_json::to_string(&payload)?,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn get(&self, incident_id: &str) -> Result<IncidentRecord> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow!("incident database lock is poisoned"))?;
        let json = connection
            .query_row(
                "SELECT record_json FROM incidents WHERE incident_id = ?1",
                [incident_id],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => anyhow!("unknown incident: {incident_id}"),
                other => other.into(),
            })?;
        serde_json::from_str(&json).context("parse stored incident")
    }

    pub fn list(&self) -> Result<Vec<IncidentRecord>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow!("incident database lock is poisoned"))?;
        let mut statement =
            connection.prepare("SELECT record_json FROM incidents ORDER BY updated_at DESC")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| {
            let json = row?;
            serde_json::from_str(&json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    json.len(),
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
    }

    pub fn pending_fabric_events(&self, limit: usize) -> Result<Vec<FabricOutboxEvent>> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow!("incident database lock is poisoned"))?;
        let mut statement = connection.prepare(
            "
            SELECT event_id, payload_json, attempt_count
            FROM fabric_outbox
            WHERE status = 'pending'
              AND (next_attempt_at IS NULL OR next_attempt_at <= ?1)
            ORDER BY occurred_at, event_id
            LIMIT ?2
            ",
        )?;
        let rows = statement.query_map(
            params![Utc::now().to_rfc3339(), i64::try_from(limit)?],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                ))
            },
        )?;
        rows.map(|row| {
            let (event_id, payload_json, attempt_count) = row?;
            let payload = serde_json::from_str(&payload_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    payload_json.len(),
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            Ok(FabricOutboxEvent {
                event_id,
                payload,
                attempt_count,
            })
        })
        .collect::<std::result::Result<Vec<_>, rusqlite::Error>>()
        .map_err(Into::into)
    }

    pub fn mark_fabric_published(&self, event_ids: &[&str]) -> Result<()> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| anyhow!("incident database lock is poisoned"))?;
        let transaction = connection.transaction()?;
        let published_at = Utc::now().to_rfc3339();
        for event_id in event_ids {
            transaction.execute(
                "
                UPDATE fabric_outbox
                SET status = 'published',
                    published_at = ?2,
                    next_attempt_at = NULL,
                    last_error = NULL
                WHERE event_id = ?1
                ",
                params![event_id, published_at],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn record_fabric_failure(
        &self,
        event_id: &str,
        error: &str,
        retry_after: Duration,
        terminal: bool,
    ) -> Result<()> {
        let next_attempt_at = if terminal {
            None
        } else {
            Some((Utc::now() + ChronoDuration::from_std(retry_after)?).to_rfc3339())
        };
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow!("incident database lock is poisoned"))?;
        connection.execute(
            "
            UPDATE fabric_outbox
            SET status = ?2,
                attempt_count = attempt_count + 1,
                next_attempt_at = ?3,
                last_error = ?4
            WHERE event_id = ?1
            ",
            params![
                event_id,
                if terminal { "failed" } else { "pending" },
                next_attempt_at,
                error,
            ],
        )?;
        Ok(())
    }

    pub fn fabric_publication_status(
        &self,
        enabled: bool,
        factory_id: &str,
    ) -> Result<FabricPublicationStatus> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow!("incident database lock is poisoned"))?;
        let (pending, published, failed, last_success_at): (u64, u64, u64, Option<String>) =
            connection.query_row(
                "
                SELECT
                    CAST(COALESCE(SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END), 0) AS INTEGER),
                    CAST(COALESCE(SUM(CASE WHEN status = 'published' THEN 1 ELSE 0 END), 0) AS INTEGER),
                    CAST(COALESCE(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0) AS INTEGER),
                    MAX(published_at)
                FROM fabric_outbox
                ",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;
        let last_error = connection
            .query_row(
                "
                SELECT last_error
                FROM fabric_outbox
                WHERE last_error IS NOT NULL
                ORDER BY occurred_at DESC
                LIMIT 1
                ",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;

        Ok(FabricPublicationStatus {
            enabled,
            factory_id: factory_id.to_string(),
            pending,
            published,
            failed,
            last_success_at: last_success_at
                .map(|value| value.parse())
                .transpose()
                .context("parse Fabric publication timestamp")?,
            last_error,
        })
    }

    pub fn reset(&self) -> Result<()> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| anyhow!("incident database lock is poisoned"))?;
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM incidents", [])?;
        transaction.execute("DELETE FROM fabric_outbox", [])?;
        transaction.execute(
            "UPDATE app_state SET value = 'true' WHERE key = 'factory_connected'",
            [],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn factory_connected(&self) -> Result<bool> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow!("incident database lock is poisoned"))?;
        let value = connection.query_row(
            "SELECT value FROM app_state WHERE key = 'factory_connected'",
            [],
            |row| row.get::<_, String>(0),
        )?;
        Ok(value == "true")
    }

    pub fn set_factory_connected(&self, connected: bool) -> Result<()> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow!("incident database lock is poisoned"))?;
        connection.execute(
            "
            INSERT INTO app_state(key, value) VALUES ('factory_connected', ?1)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            ",
            [if connected { "true" } else { "false" }],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        AuditEvent, ConnectivityState, IncidentPackage, IncidentStatus, NormalizedAlarm,
    };
    use serde_json::json;

    #[test]
    fn connectivity_state_is_persistent_and_resettable() {
        let store = IncidentStore::in_memory().unwrap();
        assert!(store.factory_connected().unwrap());
        store.set_factory_connected(false).unwrap();
        assert!(!store.factory_connected().unwrap());
        store.reset().unwrap();
        assert!(store.factory_connected().unwrap());
    }

    #[test]
    fn incident_save_transactionally_enqueues_each_governed_event_once() {
        let store = IncidentStore::in_memory().unwrap();
        let now = Utc::now();
        let record = IncidentRecord {
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
                    raw_text: "do not export".into(),
                    severity: "critical".into(),
                },
                context: json!({}),
                local_assessment: None,
                connectivity: ConnectivityState::Connected,
            },
            status: IncidentStatus::Executed,
            proposal: None,
            guard_decision: None,
            audit: vec![AuditEvent {
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
            }],
            created_at: now,
            updated_at: now,
        };

        store.save(&record).unwrap();
        store.save(&record).unwrap();

        let events = store.pending_fabric_events(10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload.factory_id, "factory-test");
        assert_eq!(events[0].payload.details["decision_id"], "decision-1");
        assert!(events[0].payload.details.get("approved_by").is_none());
        assert!(events[0].payload.details.get("raw_text").is_none());

        store.mark_fabric_published(&["event-1"]).unwrap();
        let status = store
            .fabric_publication_status(true, "factory-test")
            .unwrap();
        assert_eq!(status.pending, 0);
        assert_eq!(status.published, 1);
        assert_eq!(status.failed, 0);

        store.reset().unwrap();
        let status = store
            .fabric_publication_status(true, "factory-test")
            .unwrap();
        assert_eq!(status.published, 0);
    }
}
