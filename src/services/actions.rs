use crate::domain::{
    ActionStatus, CoordinatedRun, CreateReduceSpeedRequest, GuidanceStatus, ReduceSpeedAction,
    Scenario,
};
use anyhow::{Result, anyhow, bail};
use chrono::Utc;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

const MIN_REDUCTION_PERCENT: u8 = 5;
const MAX_REDUCTION_PERCENT: u8 = 30;

#[derive(Clone)]
struct ActionEvidence {
    scenario_id: String,
    machine_id: String,
}

#[derive(Clone, Default)]
pub struct ActionService {
    evidence: Arc<Mutex<HashMap<String, ActionEvidence>>>,
    actions: Arc<Mutex<HashMap<String, ReduceSpeedAction>>>,
}

impl ActionService {
    pub async fn register_run(&self, scenario: &Scenario, run: &CoordinatedRun) {
        let slow_is_grounded = run.slow_response.as_ref().is_some_and(|response| {
            response.success
                && !response.mock
                && response.agentic_evidence.as_ref().is_some_and(|evidence| {
                    evidence.status == "completed"
                        && !evidence.citations.is_empty()
                        && evidence.steps.iter().any(|step| {
                            matches!(
                                step.step_type.as_str(),
                                "mcp_call" | "tool_calls" | "local_retrieval"
                            )
                        })
                })
        }) && run.guidance_status == Some(GuidanceStatus::Current);
        if !slow_is_grounded {
            return;
        }
        let Some(machine_id) = run
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.machine_event.machine_id.clone())
        else {
            return;
        };
        let Some(response) = run
            .fast_response
            .as_ref()
            .filter(|response| response.success)
        else {
            return;
        };
        self.evidence.lock().await.insert(
            response.request_id.clone(),
            ActionEvidence {
                scenario_id: scenario.id.clone(),
                machine_id,
            },
        );
    }

    pub async fn create_reduce_speed(
        &self,
        request: CreateReduceSpeedRequest,
    ) -> Result<ReduceSpeedAction> {
        if !(MIN_REDUCTION_PERCENT..=MAX_REDUCTION_PERCENT).contains(&request.reduction_percent) {
            bail!(
                "reduction_percent must be between {MIN_REDUCTION_PERCENT} and {MAX_REDUCTION_PERCENT}"
            );
        }
        let evidence = self
            .evidence
            .lock()
            .await
            .get(&request.inference_request_id)
            .cloned()
            .ok_or_else(|| anyhow!("inference request is not eligible for machine action"))?;
        if evidence.scenario_id != request.scenario_id {
            bail!("scenario does not match the referenced inference request");
        }
        let action = ReduceSpeedAction {
            action_id: Uuid::new_v4().to_string(),
            scenario_id: request.scenario_id,
            inference_request_id: request.inference_request_id,
            machine_id: evidence.machine_id,
            reduction_percent: request.reduction_percent,
            connector_id: "simulated-machine-connector".into(),
            simulated: true,
            status: ActionStatus::PendingApproval,
            requested_at: Utc::now(),
            approved_at: None,
            executed_at: None,
            approved_by: None,
            outcome: None,
        };
        self.actions
            .lock()
            .await
            .insert(action.action_id.clone(), action.clone());
        Ok(action)
    }

    pub async fn approve(
        &self,
        action_id: &str,
        approved: bool,
        operator_id: String,
    ) -> Result<ReduceSpeedAction> {
        if !approved {
            bail!("explicit approval is required");
        }
        let operator_id = operator_id.trim();
        if operator_id.is_empty() {
            bail!("operator_id is required");
        }
        let mut actions = self.actions.lock().await;
        let action = actions
            .get_mut(action_id)
            .ok_or_else(|| anyhow!("unknown action request"))?;
        if action.status == ActionStatus::Executed {
            return Ok(action.clone());
        }
        let now = Utc::now();
        action.status = ActionStatus::Executed;
        action.approved_at = Some(now);
        action.executed_at = Some(now);
        action.approved_by = Some(operator_id.into());
        action.outcome = Some(format!(
            "Simulated connector accepted a {}% speed reduction request for {}",
            action.reduction_percent, action.machine_id
        ));
        Ok(action.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        AgenticRunEvidence, AgenticStepEvidence, ChatResponse, DataClassification,
        EvidenceSnapshot, GuidanceStatus, KnowledgeCitation, RequestedTarget, RoutingDecision,
        TargetType,
    };
    use chrono::Utc;

    fn eligible_run() -> (Scenario, CoordinatedRun) {
        let scenario = Scenario {
            id: "robot-bearing-alarm".into(),
            title: "Robot alarm".into(),
            description: String::new(),
            default_prompt: String::new(),
            recommended_target: RequestedTarget::Device,
            data_classification: DataClassification::LocalOnly,
            requires_fast_slow: false,
            requires_shared_context: false,
            event_id: Some("robot-17-primary".into()),
        };
        let response = ChatResponse {
            request_id: "request-1".into(),
            provider_id: "device".into(),
            target_type: TargetType::Device,
            model_id: "model".into(),
            endpoint_alias: "local".into(),
            content: "advisory".into(),
            finish_reason: Some("stop".into()),
            input_tokens: None,
            output_tokens: None,
            latency_ms: 1,
            fallback_used: false,
            success: true,
            timestamp: Utc::now(),
            confidence: None,
            raw_response_logged: false,
            mock: false,
            agentic_evidence: Some(AgenticRunEvidence {
                thread_id: "thread-1".into(),
                run_id: "run-1".into(),
                agent_id: "agent-1".into(),
                status: "completed".into(),
                steps: vec![AgenticStepEvidence {
                    step_type: "mcp_call".into(),
                    tool_name: Some("factory-manuals".into()),
                    details: serde_json::json!({}),
                }],
                citations: vec![KnowledgeCitation {
                    title: Some("Bearing maintenance guide".into()),
                    source: Some("bearing-maintenance-guide.md".into()),
                    excerpt: Some("Reduce load pending inspection.".into()),
                }],
            }),
            error: None,
        };
        let snapshot = EvidenceSnapshot {
            snapshot_id: "snapshot-1".into(),
            created_at: Utc::now(),
            machine_event: crate::domain::MachineEvent {
                machine_id: "Robot 17".into(),
                line_id: "Line A".into(),
                event_timestamp: Utc::now(),
                alarm_code: "BRG-VIB-CRIT".into(),
                vibration_level: serde_json::json!(9.2),
                bearing_temperature_c: 88.4,
                motor_current_a: Some(31.2),
                operating_state: "paused_for_inspection".into(),
                maintenance_notes: vec![],
                data_classification: DataClassification::LocalOnly,
                severity: "critical".into(),
                evidence_version: "v1".into(),
            },
            local_result: response.clone(),
            peer_machine_summary: None,
            evidence_version: "v1".into(),
            data_classification: DataClassification::LocalOnly,
        };
        let run = CoordinatedRun {
            mode: "Fast + Slow".into(),
            routing: RoutingDecision {
                requested_target: RequestedTarget::Device,
                selected_target: TargetType::Device,
                rule_id: "operator-selection".into(),
                reason: "operator selected device".into(),
            },
            escalation: None,
            snapshot: Some(snapshot),
            fast_response: Some(response.clone()),
            slow_response: Some(response.clone()),
            guidance_status: Some(GuidanceStatus::Current),
        };
        (scenario, run)
    }

    #[tokio::test]
    async fn action_requires_registered_inference_and_explicit_approval() {
        let service = ActionService::default();
        let (scenario, run) = eligible_run();
        service.register_run(&scenario, &run).await;
        let action = service
            .create_reduce_speed(CreateReduceSpeedRequest {
                scenario_id: scenario.id,
                inference_request_id: "request-1".into(),
                reduction_percent: 20,
            })
            .await
            .unwrap();
        assert_eq!(action.status, ActionStatus::PendingApproval);
        assert!(
            service
                .approve(&action.action_id, false, "operator".into())
                .await
                .is_err()
        );
        let executed = service
            .approve(&action.action_id, true, "operator".into())
            .await
            .unwrap();
        assert_eq!(executed.status, ActionStatus::Executed);
        let repeated = service
            .approve(&action.action_id, true, "operator".into())
            .await
            .unwrap();
        assert_eq!(repeated.executed_at, executed.executed_at);
    }

    #[tokio::test]
    async fn action_rejects_out_of_range_reduction() {
        let service = ActionService::default();
        let result = service
            .create_reduce_speed(CreateReduceSpeedRequest {
                scenario_id: "robot-bearing-alarm".into(),
                inference_request_id: "missing".into(),
                reduction_percent: 50,
            })
            .await;
        assert!(result.is_err());
    }
}
