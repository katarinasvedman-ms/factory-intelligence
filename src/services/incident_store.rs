use crate::domain::IncidentRecord;
use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, params};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct IncidentStore {
    connection: Arc<Mutex<Connection>>,
}

impl IncidentStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
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
        Self::from_connection(connection)
    }

    pub fn open_default() -> Result<Self> {
        let path = std::env::var("FACTORY_DB_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("data/governed-floor.db"));
        Self::open(path)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
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
            ",
        )?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn save(&self, record: &IncidentRecord) -> Result<()> {
        let json = serde_json::to_string(record)?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow!("incident database lock is poisoned"))?;
        connection.execute(
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

    pub fn reset(&self) -> Result<()> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow!("incident database lock is poisoned"))?;
        connection.execute("DELETE FROM incidents", [])?;
        connection.execute(
            "UPDATE app_state SET value = 'true' WHERE key = 'factory_connected'",
            [],
        )?;
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

    #[test]
    fn connectivity_state_is_persistent_and_resettable() {
        let store = IncidentStore::in_memory().unwrap();
        assert!(store.factory_connected().unwrap());
        store.set_factory_connected(false).unwrap();
        assert!(!store.factory_connected().unwrap());
        store.reset().unwrap();
        assert!(store.factory_connected().unwrap());
    }
}
