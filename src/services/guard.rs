use crate::domain::{
    ActionProposal, ConnectivityState, GuardDecision, IncidentRecord, IncidentStatus,
};
use chrono::{Duration, Utc};
use uuid::Uuid;

#[derive(Clone, Default)]
pub struct GuardService;

impl GuardService {
    pub fn evaluate(
        &self,
        record: &IncidentRecord,
        proposal: &ActionProposal,
        operator_id: &str,
    ) -> GuardDecision {
        let reject = |reason: String| GuardDecision {
            decision_id: Uuid::new_v4().to_string(),
            proposal_id: proposal.proposal_id.clone(),
            incident_id: record.incident.incident_id.clone(),
            permitted: false,
            reason,
            checked_at: Utc::now(),
            approved_by: Some(operator_id.trim().into()),
            executed: false,
            outcome: None,
        };
        if operator_id.trim().is_empty() {
            return reject("An identified operator is required.".into());
        }
        if record.status != IncidentStatus::AwaitingApproval {
            return reject("The incident is not awaiting an operator decision.".into());
        }
        if record.incident.connectivity != ConnectivityState::Connected {
            return reject("The machine is not connected for governed execution.".into());
        }
        if proposal.incident_id != record.incident.incident_id {
            return reject("The proposal does not reference the current incident.".into());
        }
        if Utc::now().signed_duration_since(proposal.created_at) > Duration::minutes(10) {
            return reject(
                "The proposal is stale and must be refreshed against current machine evidence."
                    .into(),
            );
        }
        if record.incident.context.pointer("/signals").is_none() {
            return reject("Current machine measurements are unavailable.".into());
        }
        if proposal.proposed_action.action_id != "reduce_speed" {
            return reject(format!(
                "Action '{}' is not on the governed allowlist.",
                proposal.proposed_action.action_id
            ));
        }
        let Some(percent) = proposal
            .proposed_action
            .parameters
            .get("reduction_percent")
            .and_then(serde_json::Value::as_u64)
        else {
            return reject("The speed reduction parameter is missing or invalid.".into());
        };
        if !(5..=30).contains(&percent) {
            return reject(format!(
                "Requested speed reduction of {percent}% exceeds the governed 5–30% limit."
            ));
        }
        GuardDecision {
            decision_id: Uuid::new_v4().to_string(),
            proposal_id: proposal.proposal_id.clone(),
            incident_id: record.incident.incident_id.clone(),
            permitted: true,
            reason: "Action is allowlisted, bounded, current, and explicitly approved.".into(),
            checked_at: Utc::now(),
            approved_by: Some(operator_id.trim().into()),
            executed: true,
            outcome: Some(format!(
                "Simulated connector accepted a {percent}% speed reduction for {}.",
                record.incident.machine_id
            )),
        }
    }

    pub fn reject(
        &self,
        record: &IncidentRecord,
        proposal: &ActionProposal,
        operator_id: &str,
    ) -> GuardDecision {
        GuardDecision {
            decision_id: Uuid::new_v4().to_string(),
            proposal_id: proposal.proposal_id.clone(),
            incident_id: record.incident.incident_id.clone(),
            permitted: false,
            reason: "Operator rejected the proposed action.".into(),
            checked_at: Utc::now(),
            approved_by: Some(operator_id.trim().into()),
            executed: false,
            outcome: None,
        }
    }
}
