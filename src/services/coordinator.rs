use crate::{
    domain::{
        CoordinatedRun, EscalationDecision, EvidenceSnapshot, GuidanceStatus, Message,
        RequestedTarget, RoutingDecision, Scenario, TargetType,
    },
    services::{InferenceService, RoutingPolicy, ScenarioService},
};
use anyhow::{Result, anyhow};
use chrono::Utc;
use uuid::Uuid;

#[derive(Clone)]
pub struct FastSlowCoordinator {
    scenarios: ScenarioService,
    routing: RoutingPolicy,
    inference: InferenceService,
}

impl FastSlowCoordinator {
    pub fn new(
        scenarios: ScenarioService,
        routing: RoutingPolicy,
        inference: InferenceService,
    ) -> Self {
        Self {
            scenarios,
            routing,
            inference,
        }
    }

    pub async fn run(
        &self,
        scenario: &Scenario,
        requested_target: RequestedTarget,
        messages: Vec<Message>,
        allow_fallback: Option<bool>,
    ) -> Result<CoordinatedRun> {
        if !scenario.requires_fast_slow {
            let decision = self.routing.select(scenario, requested_target)?;
            let response = self
                .inference
                .run(scenario, &decision, messages, allow_fallback)
                .await?;
            let fast = decision.selected_target == TargetType::Device;
            return Ok(CoordinatedRun {
                mode: if fast { "Fast path" } else { "Slow path" }.into(),
                routing: decision,
                escalation: None,
                snapshot: None,
                fast_response: fast.then_some(response.clone()),
                slow_response: (!fast).then_some(response),
                guidance_status: Some(GuidanceStatus::Current),
            });
        }

        let event = self
            .scenarios
            .event(scenario.event_id.as_deref())
            .ok_or_else(|| anyhow!("fast/slow scenario is missing its machine event"))?;
        let fast_routing = RoutingDecision {
            requested_target,
            selected_target: TargetType::Device,
            rule_id: "fast-path-first".into(),
            reason: "Primary machine event always starts on the device".into(),
        };
        let fast_response = self
            .inference
            .run(scenario, &fast_routing, messages.clone(), Some(false))
            .await?;
        if requested_target == RequestedTarget::Device {
            let snapshot_id = Uuid::new_v4().to_string();
            let snapshot = EvidenceSnapshot {
                snapshot_id: snapshot_id.clone(),
                created_at: Utc::now(),
                machine_event: event.clone(),
                local_result: fast_response.clone(),
                peer_machine_summary: None,
                evidence_version: event.evidence_version.clone(),
                data_classification: event.data_classification,
            };
            return Ok(CoordinatedRun {
                mode: "Fast path (device constrained)".into(),
                routing: fast_routing,
                escalation: Some(EscalationDecision {
                    escalate: false,
                    target: Some(TargetType::Edge),
                    rule_id: "operator-device-only".into(),
                    reason:
                        "Operator selected device-only execution; shared-context analysis was not run"
                            .into(),
                    snapshot_id: Some(snapshot_id),
                }),
                snapshot: Some(snapshot),
                fast_response: Some(fast_response),
                slow_response: None,
                guidance_status: Some(GuidanceStatus::RevalidationRequired),
            });
        }
        let escalate = event.severity == "critical"
            || scenario.requires_shared_context
            || fast_response
                .confidence
                .is_some_and(|confidence| confidence < 0.8);

        if !escalate {
            return Ok(CoordinatedRun {
                mode: "Fast path".into(),
                routing: fast_routing,
                escalation: Some(EscalationDecision {
                    escalate: false,
                    target: None,
                    rule_id: "routine-local-result".into(),
                    reason: "Local evidence is sufficient for this advisory response".into(),
                    snapshot_id: None,
                }),
                snapshot: None,
                fast_response: Some(fast_response),
                slow_response: None,
                guidance_status: Some(GuidanceStatus::Current),
            });
        }

        let snapshot_id = Uuid::new_v4().to_string();
        let snapshot = EvidenceSnapshot {
            snapshot_id: snapshot_id.clone(),
            created_at: Utc::now(),
            machine_event: event.clone(),
            local_result: fast_response.clone(),
            peer_machine_summary: Some(self.scenarios.peer_summary(&event.line_id)),
            evidence_version: event.evidence_version.clone(),
            data_classification: event.data_classification,
        };
        let escalation = EscalationDecision {
            escalate: true,
            target: Some(TargetType::Edge),
            rule_id: "critical-or-shared-context".into(),
            reason: "Critical severity, shared context, or insufficient local confidence".into(),
            snapshot_id: Some(snapshot_id),
        };

        let mut slow_messages = messages;
        let mut redacted_snapshot = serde_json::to_value(&snapshot)?;
        if let Some(content) = redacted_snapshot.pointer_mut("/local_result/content") {
            *content = serde_json::Value::String("[included separately]".into());
        }
        slow_messages.push(Message {
            role: "assistant".into(),
            content: format!(
                "Fast-path advisory result: {}\nEvidence snapshot: {}",
                fast_response.content, redacted_snapshot
            ),
        });
        slow_messages.push(Message {
            role: "user".into(),
            content: "Correlate the immutable synthetic evidence across the factory line. Return advisory guidance only; do not authorize equipment actuation.".into(),
        });
        let slow_routing = RoutingDecision {
            requested_target,
            selected_target: TargetType::Edge,
            rule_id: escalation.rule_id.clone(),
            reason: escalation.reason.clone(),
        };
        let slow_response = self
            .inference
            .run(scenario, &slow_routing, slow_messages, allow_fallback)
            .await?;
        let current = self.scenarios.event(scenario.event_id.as_deref());
        let guidance = if current
            .as_ref()
            .is_some_and(|event| event.evidence_version == snapshot.evidence_version)
        {
            GuidanceStatus::Current
        } else {
            GuidanceStatus::Stale
        };
        Ok(CoordinatedRun {
            mode: "Fast + Slow".into(),
            routing: slow_routing,
            escalation: Some(escalation),
            snapshot: Some(snapshot),
            fast_response: Some(fast_response),
            slow_response: Some(slow_response),
            guidance_status: Some(guidance),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::load_settings, providers::ProviderRegistry};
    use std::sync::Arc;

    #[tokio::test]
    async fn robot_bearing_runs_fast_and_slow() {
        let settings = load_settings().unwrap();
        let registry = Arc::new(ProviderRegistry::new(&settings).unwrap());
        let scenarios = ScenarioService::load().unwrap();
        let coordinator = FastSlowCoordinator::new(
            scenarios.clone(),
            RoutingPolicy,
            InferenceService::new(settings.application, registry),
        );
        let scenario = scenarios.get("robot-bearing-alarm").unwrap();
        let result = coordinator
            .run(
                &scenario,
                RequestedTarget::Auto,
                vec![Message {
                    role: "user".into(),
                    content: scenario.default_prompt.clone(),
                }],
                Some(false),
            )
            .await
            .unwrap();
        assert_eq!(result.mode, "Fast + Slow");
        assert!(result.fast_response.is_some());
        assert!(result.slow_response.is_some());
        assert_eq!(result.guidance_status, Some(GuidanceStatus::Current));
    }
}
