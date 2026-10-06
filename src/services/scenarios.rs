use crate::domain::{MachineEvent, Scenario};
use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, fs};

#[derive(Deserialize)]
struct IncidentRow {
    id: String,
    event: MachineEvent,
}

#[derive(Clone)]
pub struct ScenarioService {
    scenarios: HashMap<String, Scenario>,
    events: HashMap<String, MachineEvent>,
}

impl ScenarioService {
    pub fn load() -> Result<Self> {
        let scenario_yaml =
            fs::read_to_string("config/scenarios.yaml").context("read scenario configuration")?;
        let scenarios: Vec<Scenario> =
            serde_yaml::from_str(&scenario_yaml).context("parse scenarios")?;
        let scenarios = scenarios
            .into_iter()
            .map(|scenario| (scenario.id.clone(), scenario))
            .collect();

        let incidents = fs::read_to_string("data/synthetic_factory_incidents.jsonl")
            .context("read synthetic incidents")?;
        let mut events = HashMap::new();
        for line in incidents.lines().filter(|line| !line.trim().is_empty()) {
            let row: IncidentRow =
                serde_json::from_str(line).context("parse synthetic incident")?;
            events.insert(row.id, row.event);
        }
        Ok(Self { scenarios, events })
    }

    pub fn list(&self) -> Vec<Scenario> {
        let mut scenarios = self.scenarios.values().cloned().collect::<Vec<_>>();
        scenarios.sort_by(|a, b| a.id.cmp(&b.id));
        scenarios
    }

    pub fn get(&self, id: &str) -> Result<Scenario> {
        self.scenarios
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("unknown scenario: {id}"))
    }

    pub fn event(&self, id: Option<&str>) -> Option<MachineEvent> {
        id.and_then(|id| self.events.get(id)).cloned()
    }

    pub fn peer_summary(&self, line_id: &str) -> Value {
        let peers = self
            .events
            .values()
            .filter(|event| event.line_id == line_id)
            .collect::<Vec<_>>();
        json!({
            "line_id": line_id,
            "event_count": peers.len(),
            "machines": peers.iter().map(|event| &event.machine_id).collect::<Vec<_>>(),
            "critical_events": peers.iter().filter(|event| event.severity == "critical").count(),
            "synthetic": true
        })
    }
}
