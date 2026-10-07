use crate::{
    domain::{
        ActionProposal, AuditEvent, ConnectivityState, IncidentPackage, IncidentRecord,
        IncidentStatus, KnowledgeCitation, LocalAssessment, ManagementBriefMode,
        ManagementBriefPreviewRequest, Message, ProposedAction, RequestedTarget, RiskLevel,
    },
    services::{
        FastSlowCoordinator, GuardService, IncidentStore, ScenarioService, VendorSimulator,
    },
};
use anyhow::{Result, anyhow};
use chrono::Utc;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct GovernedFloorService {
    store: IncidentStore,
    scenarios: ScenarioService,
    coordinator: FastSlowCoordinator,
    vendors: VendorSimulator,
    guard: GuardService,
    management_previews: Arc<Mutex<HashMap<String, Value>>>,
}

impl GovernedFloorService {
    pub fn new(
        store: IncidentStore,
        scenarios: ScenarioService,
        coordinator: FastSlowCoordinator,
    ) -> Self {
        Self {
            store,
            scenarios,
            coordinator,
            vendors: VendorSimulator,
            guard: GuardService,
            management_previews: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn list(&self) -> Result<Vec<IncidentRecord>> {
        self.store.list()
    }

    pub fn get(&self, incident_id: &str) -> Result<IncidentRecord> {
        self.store.get(incident_id)
    }

    pub fn reset(&self) -> Result<()> {
        self.store.reset()?;
        self.management_previews
            .lock()
            .map_err(|_| anyhow!("management preview lock is poisoned"))?
            .clear();
        Ok(())
    }

    pub fn factory_connected(&self) -> Result<bool> {
        self.store.factory_connected()
    }

    pub async fn set_factory_connected(&self, connected: bool) -> Result<Vec<IncidentRecord>> {
        self.store.set_factory_connected(connected)?;
        if !connected {
            return Ok(vec![]);
        }
        let queued = self
            .store
            .list()?
            .into_iter()
            .filter(|record| {
                record.proposal.is_none()
                    && record
                        .incident
                        .context
                        .get("queued_for_sync")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
            })
            .collect::<Vec<_>>();
        let mut synchronized = Vec::with_capacity(queued.len());
        for record in queued {
            synchronized.push(self.synchronize_incident(record).await?);
        }
        Ok(synchronized)
    }

    pub async fn run_demo(&self, scenario_id: &str) -> Result<Vec<IncidentRecord>> {
        let alarms = self.vendors.alarms_for(scenario_id)?;
        let packages = alarms
            .iter()
            .cloned()
            .map(|alarm| self.vendors.normalize(alarm, &alarms))
            .collect::<Result<Vec<_>>>()?;
        match scenario_id {
            "routine-local" => self.run_routine(packages).await,
            "cross-vendor-cascade" => self.run_cascade(packages).await,
            "unsafe-action" => self.run_unsafe(packages),
            "network-loss" => self.run_offline(packages).await,
            _ => Err(anyhow!("unknown guided demo scenario: {scenario_id}")),
        }
    }

    pub fn approve(&self, incident_id: &str, operator_id: &str) -> Result<IncidentRecord> {
        let mut record = self.store.get(incident_id)?;
        let proposal = record
            .proposal
            .clone()
            .ok_or_else(|| anyhow!("incident has no action proposal"))?;
        if proposal.sources.is_empty() {
            return Err(anyhow!(
                "incident proposal is ungrounded and cannot be submitted to the Guard"
            ));
        }
        let decision = self.guard.evaluate(&record, &proposal, operator_id);
        record.status = if decision.permitted {
            IncidentStatus::Executed
        } else {
            IncidentStatus::Rejected
        };
        record.guard_decision = Some(decision.clone());
        record.updated_at = Utc::now();
        record.audit.push(audit_event(
            incident_id,
            if decision.permitted {
                "guard_executed"
            } else {
                "guard_rejected"
            },
            &decision.reason,
            json!({
                "decision_id": decision.decision_id,
                "approved_by": decision.approved_by,
                "executed": decision.executed,
                "outcome": decision.outcome
            }),
        ));
        self.store.save(&record)?;
        if decision.executed {
            self.mark_related_incidents_monitoring(&record, &decision.decision_id)?;
        }
        Ok(record)
    }

    pub fn reject(&self, incident_id: &str, operator_id: &str) -> Result<IncidentRecord> {
        let mut record = self.store.get(incident_id)?;
        let proposal = record
            .proposal
            .clone()
            .ok_or_else(|| anyhow!("incident has no action proposal"))?;
        let decision = self.guard.reject(&record, &proposal, operator_id);
        record.status = IncidentStatus::Rejected;
        record.guard_decision = Some(decision.clone());
        record.updated_at = Utc::now();
        record.audit.push(audit_event(
            incident_id,
            "operator_rejected",
            &decision.reason,
            json!({"approved_by": decision.approved_by}),
        ));
        self.store.save(&record)?;
        Ok(record)
    }

    pub fn preview_management_brief(
        &self,
        request: ManagementBriefPreviewRequest,
    ) -> Result<Value> {
        if request.from >= request.to {
            return Err(anyhow!("report start must be before report end"));
        }
        if request.to.signed_duration_since(request.from).num_days() > 31 {
            return Err(anyhow!("management report range cannot exceed 31 days"));
        }
        let mut records = self
            .store
            .list()?
            .into_iter()
            .filter(|record| {
                record.incident.timestamp >= request.from && record.incident.timestamp <= request.to
            })
            .collect::<Vec<_>>();
        records.sort_by_key(|record| record.incident.timestamp);
        if matches!(request.mode, ManagementBriefMode::LatestExecuted) {
            let executed = records
                .iter()
                .rev()
                .find(|record| record.status == IncidentStatus::Executed)
                .cloned()
                .ok_or_else(|| anyhow!("the selected range has no executed incident"))?;
            let related_id = executed
                .audit
                .iter()
                .find(|event| event.event_type == "cross_vendor_correlation")
                .and_then(|event| event.details.get("related_incident_id"))
                .and_then(serde_json::Value::as_str);
            records.retain(|record| {
                record.incident.incident_id == executed.incident.incident_id
                    || related_id == Some(record.incident.incident_id.as_str())
            });
        }
        if records.is_empty() {
            return Err(anyhow!("the selected report scope contains no incidents"));
        }
        let approved_incidents = records
            .iter()
            .map(|record| {
                let governed_action = record.proposal.as_ref().map(|proposal| {
                    json!({
                        "action": proposal.proposed_action.action_id,
                        "parameters": proposal.proposed_action.parameters,
                        "risk_level": proposal.proposed_action.risk_level,
                        "guard_permitted": record.guard_decision.as_ref().map(|decision| decision.permitted),
                        "executed": record.guard_decision.as_ref().map(|decision| decision.executed)
                    })
                });
                json!({
                    "incident_id": record.incident.incident_id,
                    "timestamp": record.incident.timestamp,
                    "line_id": record.incident.line_id,
                    "machine_id": record.incident.machine_id,
                    "vendor_profile": record.incident.vendor_profile,
                    "normalized_alarm": record.incident.alarm.code,
                    "severity": record.incident.alarm.severity,
                    "status": record.status,
                    "connectivity": record.incident.connectivity,
                    "governed_action": governed_action
                })
            })
            .collect::<Vec<_>>();
        let preview_id = Uuid::new_v4().to_string();
        let payload = json!({
            "purpose": "post_incident_management_brief",
            "classification": "cloud_allowed",
            "synthetic": true,
            "scope": {
                "from": request.from,
                "to": request.to,
                "mode": request.mode
            },
            "incidents": approved_incidents
        });
        let exclusions = vec![
            "Raw telemetry and high-frequency sensor samples",
            "Raw vendor alarm text",
            "Machine-local and factory-agent reasoning",
            "Retrieved manual excerpts and citations",
            "Operator identity",
            "Credentials, endpoints, and connector details",
        ];
        let preview = json!({
            "preview_id": preview_id,
            "created_at": Utc::now(),
            "incident_count": records.len(),
            "payload": payload,
            "exclusions": exclusions,
            "policy": {
                "target": "cloud",
                "model_role": "Expert reliability analysis",
                "machine_authority": false,
                "generation_requires_preview_id": true
            }
        });
        self.management_previews
            .lock()
            .map_err(|_| anyhow!("management preview lock is poisoned"))?
            .insert(preview_id, preview.clone());
        Ok(preview)
    }

    pub async fn create_management_brief(&self, preview_id: &str) -> Result<Value> {
        let preview = self
            .management_previews
            .lock()
            .map_err(|_| anyhow!("management preview lock is poisoned"))?
            .get(preview_id)
            .cloned()
            .ok_or_else(|| anyhow!("unknown or expired management preview"))?;
        let approved_snapshot = preview
            .get("payload")
            .cloned()
            .ok_or_else(|| anyhow!("management preview has no approved payload"))?;
        let scenario = self.scenarios.get("management-summary")?;
        let prompt = format!(
            "You are supporting reliability and plant leadership after synthetic factory incidents. \
             Use only the approved facts below. Return exactly three concise plain-text bullet \
             recommendations for longer-term reliability improvement. Do not restate the \
             incident. Do not infer safety, regulatory, customer, financial, downtime-duration, \
             or production-volume impact. Maximum 90 words total.\n\nApproved facts:\n{}",
            serde_json::to_string_pretty(&approved_snapshot)?
        );
        let run = self
            .coordinator
            .run(
                &scenario,
                RequestedTarget::Cloud,
                vec![Message {
                    role: "user".into(),
                    content: prompt,
                }],
                Some(false),
            )
            .await?;
        let expert = run
            .slow_response
            .ok_or_else(|| anyhow!("the Expert path returned no response"))?;
        if !expert.success {
            return Err(anyhow!(
                "the Expert path failed: {}",
                expert
                    .error
                    .map(|error| error.message)
                    .unwrap_or_else(|| "unknown error".into())
            ));
        }
        let recommendations = expert_recommendations(&expert.content);
        let incidents = approved_snapshot
            .get("incidents")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| anyhow!("approved payload has no incidents"))?;
        let mut lines = HashSet::new();
        let mut status_counts = BTreeMap::<String, usize>::new();
        let mut outcomes = Vec::new();
        for incident in incidents {
            let line = incident
                .get("line_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Unknown line");
            let machine = incident
                .get("machine_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Unknown machine");
            let alarm = incident
                .get("normalized_alarm")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown alarm");
            let status = incident
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            lines.insert(line.to_string());
            *status_counts.entry(status.to_string()).or_default() += 1;
            outcomes.push(format!("{machine}: {alarm} — {status}."));
        }
        let executed_count = status_counts.get("executed").copied().unwrap_or_default();
        let monitoring_count = status_counts.get("monitoring").copied().unwrap_or_default();
        let rejected_count = status_counts.get("rejected").copied().unwrap_or_default();
        let line_count = lines.len();
        let incident_count = incidents.len();
        let executive_summary = format!(
            "{incident_count} governed incident{} across {line_count} production line{} were included in the approved reporting scope. {executed_count} action{} executed, {monitoring_count} incident{} remain under monitoring, and {rejected_count} proposal{} were rejected by governance.",
            if incident_count == 1 { "" } else { "s" },
            if line_count == 1 { "" } else { "s" },
            if executed_count == 1 {
                " was"
            } else {
                "s were"
            },
            if monitoring_count == 1 { "" } else { "s" },
            if rejected_count == 1 { "" } else { "s" }
        );
        Ok(json!({
            "brief_id": Uuid::new_v4().to_string(),
            "preview_id": preview_id,
            "generated_at": Utc::now(),
            "title": "Governed incident management brief",
            "executive_summary": executive_summary,
            "summary": {
                "incident_count": incident_count,
                "line_count": line_count,
                "status_counts": status_counts
            },
            "incident_outcomes": outcomes,
            "recommended_follow_up": [
                "Confirm recovery evidence before closing incidents that remain under monitoring.",
                "Review rejected proposals to verify that operating procedures and action limits remain appropriate.",
                "Record validated root causes and repair outcomes in the maintenance system before publishing new factory knowledge."
            ],
            "expert_recommendations": recommendations,
            "governance": {
                "classification": "cloud_allowed",
                "snapshot": "Exact server-side preview payload",
                "preview_id": preview_id,
                "target": "cloud",
                "model_id": expert.model_id,
                "request_id": expert.request_id,
                "machine_authority": false,
            },
            "approved_payload": approved_snapshot,
            "exclusions": preview["exclusions"]
        }))
    }

    fn mark_related_incidents_monitoring(
        &self,
        upstream: &IncidentRecord,
        decision_id: &str,
    ) -> Result<()> {
        let related_ids = upstream
            .audit
            .iter()
            .filter(|event| event.event_type == "cross_vendor_correlation")
            .filter_map(|event| {
                event
                    .details
                    .get("related_incident_id")
                    .and_then(serde_json::Value::as_str)
            })
            .collect::<Vec<_>>();
        for related_id in related_ids {
            let mut related = self.store.get(related_id)?;
            if related.status != IncidentStatus::Escalated {
                continue;
            }
            related.status = IncidentStatus::Monitoring;
            related.updated_at = Utc::now();
            related.audit.push(audit_event(
                related_id,
                "upstream_action_executed",
                "The upstream action executed successfully. Monitor downstream product flow before resolving this incident.",
                json!({
                    "upstream_incident_id": upstream.incident.incident_id,
                    "decision_id": decision_id,
                    "expected_response": "product flow resumes after the upstream press stabilizes"
                }),
            ));
            self.store.save(&related)?;
        }
        Ok(())
    }

    async fn run_routine(&self, packages: Vec<IncidentPackage>) -> Result<Vec<IncidentRecord>> {
        let mut package = packages
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("routine scenario produced no alarm"))?;
        let scenario = self.scenarios.get("single-machine-alarm")?;
        let prompt = incident_prompt(&package, "Assess this alarm locally.");
        let run = self
            .coordinator
            .run(
                &scenario,
                RequestedTarget::Device,
                vec![Message {
                    role: "user".into(),
                    content: prompt,
                }],
                Some(false),
            )
            .await?;
        let response = run
            .fast_response
            .ok_or_else(|| anyhow!("routine scenario returned no local assessment"))?;
        let vibration_mm_s = package
            .context
            .pointer("/signals/vibration_mm_s")
            .and_then(Value::as_f64)
            .unwrap_or_default();
        let bearing_temperature_c = package
            .context
            .pointer("/signals/bearing_temperature_c")
            .and_then(Value::as_f64)
            .unwrap_or_default();
        let advisory_threshold_mm_s = 4.5;
        let escalation_threshold_mm_s = 7.1;
        let temperature_escalation_c = 75.0;
        let percent_above_advisory =
            ((vibration_mm_s / advisory_threshold_mm_s - 1.0) * 100.0).round();
        package.context["local_triage"] = json!({
            "condition": format!(
                "Vibration is {:.0}% above the {:.1} mm/s advisory threshold.",
                percent_above_advisory,
                advisory_threshold_mm_s
            ),
            "vibration_mm_s": vibration_mm_s,
            "bearing_temperature_c": bearing_temperature_c,
            "recommended_action": "Inspect bearing mounting, sensor seating, and lubrication within 30 minutes.",
            "operating_guidance": "Continue at current speed under observation; no automatic speed change is authorized.",
            "escalation_rule": format!(
                "Escalate if vibration reaches {:.1} mm/s, bearing temperature reaches {:.0} C, or the trend accelerates.",
                escalation_threshold_mm_s,
                temperature_escalation_c
            ),
            "decision": "Local advisory - inspection required, factory escalation not yet required.",
            "value": "The machine converted a vendor alarm into a bounded local response plan without waiting for factory or cloud analysis."
        });
        package.local_assessment = Some(LocalAssessment {
            summary: format!(
                "Vibration is {:.1} mm/s and bearing temperature is {:.0} C. Continue at current speed under observation, inspect the bearing within 30 minutes, and escalate at {:.1} mm/s vibration or {:.0} C.",
                vibration_mm_s,
                bearing_temperature_c,
                escalation_threshold_mm_s,
                temperature_escalation_c
            ),
            candidate_action_id: Some("request_inspection".into()),
            confidence: response.confidence,
            model_id: response.model_id,
        });
        let now = Utc::now();
        let record = IncidentRecord {
            audit: vec![
                audit_event(
                    &package.incident_id,
                    "alarm_normalized",
                    "Vendor alarm normalized into the governed incident contract.",
                    json!({"vendor_profile": package.vendor_profile}),
                ),
                audit_event(
                    &package.incident_id,
                    "local_assessment_completed",
                    "The machine-local fast agent completed a bounded assessment and recommended inspection.",
                    json!({"escalated": false, "candidate_action_id": "request_inspection"}),
                ),
            ],
            incident: package,
            status: IncidentStatus::LocallyAssessed,
            proposal: None,
            guard_decision: None,
            created_at: now,
            updated_at: now,
        };
        self.store.save(&record)?;
        Ok(vec![record])
    }

    async fn run_cascade(&self, mut packages: Vec<IncidentPackage>) -> Result<Vec<IncidentRecord>> {
        if packages.len() != 2 {
            return Err(anyhow!("cascade scenario requires two vendor alarms"));
        }
        let mut upstream = packages.remove(0);
        let downstream = packages.remove(0);
        let local_scenario = self.scenarios.get("single-machine-alarm")?;
        let prompt = incident_prompt(
            &upstream,
            "Correlate the upstream stop with the related downstream alarm. Return concise operator-facing guidance in at most 180 words. Do not narrate planning or tool calls. Do not emit [cite:N] markers because citations are rendered separately. Identify the likely root cause, the immediate inspection priority, and only a governed action.",
        );
        let local_run = self
            .coordinator
            .run(
                &local_scenario,
                RequestedTarget::Device,
                vec![Message {
                    role: "user".into(),
                    content: incident_prompt(
                        &upstream,
                        "Assess the upstream machine alarm locally and recommend only a bounded next step.",
                    ),
                }],
                Some(false),
            )
            .await?;
        let fast = local_run
            .fast_response
            .ok_or_else(|| anyhow!("cascade scenario returned no local assessment"))?;
        upstream.local_assessment = Some(LocalAssessment {
            summary: fast.content.clone(),
            candidate_action_id: Some("reduce_speed".into()),
            confidence: fast.confidence,
            model_id: fast.model_id.clone(),
        });
        let factory_scenario = self.scenarios.get("cross-machine-correlation")?;
        let factory_run = self
            .coordinator
            .run(
                &factory_scenario,
                RequestedTarget::Edge,
                vec![
                    Message {
                        role: "user".into(),
                        content: prompt,
                    },
                    Message {
                        role: "assistant".into(),
                        content: format!("Machine-local assessment: {}", fast.content),
                    },
                ],
                Some(false),
            )
            .await?;
        let slow = factory_run
            .slow_response
            .ok_or_else(|| anyhow!("cascade scenario returned no factory proposal"))?;
        if !slow.success {
            return Err(anyhow!(
                "factory correlation failed: {}",
                slow.error
                    .map(|error| error.message)
                    .unwrap_or_else(|| "unknown error".into())
            ));
        }
        let sources = slow
            .agentic_evidence
            .as_ref()
            .map(|evidence| evidence.citations.clone())
            .unwrap_or_default();
        let grounded = !sources.is_empty();
        let proposal = ActionProposal {
            proposal_id: Uuid::new_v4().to_string(),
            incident_id: upstream.incident_id.clone(),
            root_cause_hypothesis:
                "The upstream press drive trip caused the downstream packer starvation alarm."
                    .into(),
            operator_summary: "Press 04 stopped after a bearing-vibration trip. Packer 12 then reported loss of upstream product flow, making the packer alarm a likely downstream effect. Inspect the Press 04 bearing, sensors, mounting and lubrication condition before restart. A bounded 15% speed reduction is available for operator review.".into(),
            reasoning: slow.content,
            proposed_action: ProposedAction {
                action_id: "reduce_speed".into(),
                parameters: json!({"reduction_percent": 15}),
                risk_level: RiskLevel::Medium,
            },
            sources,
            alternatives: vec![ProposedAction {
                action_id: "request_inspection".into(),
                parameters: json!({"priority": "urgent"}),
                risk_level: RiskLevel::Low,
            }],
            created_at: Utc::now(),
        };
        let now = Utc::now();
        let upstream_record = IncidentRecord {
            audit: vec![
                audit_event(
                    &upstream.incident_id,
                    "alarm_normalized",
                    "Northstar alarm normalized into the governed incident contract.",
                    json!({"vendor_profile": upstream.vendor_profile}),
                ),
                audit_event(
                    &upstream.incident_id,
                    "cross_vendor_correlation",
                    "The factory correlator linked the Northstar stop to the Contoso starvation alarm.",
                    json!({"related_incident_id": downstream.incident_id}),
                ),
                audit_event(
                    &upstream.incident_id,
                    if grounded {
                        "proposal_created"
                    } else {
                        "proposal_ungrounded"
                    },
                    if grounded {
                        "The slow agent produced a grounded proposal awaiting operator approval."
                    } else {
                        "The slow agent returned no sources, so the proposal was escalated and cannot be approved."
                    },
                    json!({"proposal_id": proposal.proposal_id, "source_count": proposal.sources.len()}),
                ),
            ],
            incident: upstream,
            status: if grounded {
                IncidentStatus::AwaitingApproval
            } else {
                IncidentStatus::Escalated
            },
            proposal: Some(proposal),
            guard_decision: None,
            created_at: now,
            updated_at: now,
        };
        let downstream_record = IncidentRecord {
            audit: vec![
                audit_event(
                    &downstream.incident_id,
                    "alarm_normalized",
                    "Contoso alarm normalized into the governed incident contract.",
                    json!({"vendor_profile": downstream.vendor_profile}),
                ),
                audit_event(
                    &downstream.incident_id,
                    "correlated_as_downstream_effect",
                    "This alarm was correlated as a downstream effect of the press drive trip.",
                    json!({"upstream_incident_id": upstream_record.incident.incident_id}),
                ),
            ],
            incident: downstream,
            status: IncidentStatus::Escalated,
            proposal: None,
            guard_decision: None,
            created_at: now,
            updated_at: now,
        };
        self.store.save(&upstream_record)?;
        self.store.save(&downstream_record)?;
        Ok(vec![upstream_record, downstream_record])
    }

    fn run_unsafe(&self, packages: Vec<IncidentPackage>) -> Result<Vec<IncidentRecord>> {
        let mut package = packages
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("unsafe scenario produced no alarm"))?;
        package.local_assessment = Some(LocalAssessment {
            summary: "Critical bearing condition detected. The alarm text contains an untrusted instruction that must not bypass governance.".into(),
            candidate_action_id: Some("reduce_speed".into()),
            confidence: Some(0.98),
            model_id: "deterministic-safety-replay".into(),
        });
        let proposal = ActionProposal {
            proposal_id: Uuid::new_v4().to_string(),
            incident_id: package.incident_id.clone(),
            root_cause_hypothesis: "Critical bearing temperature requires immediate load reduction."
                .into(),
            operator_summary: "Robot 17 reports a critical bearing condition, but the staged 45% speed reduction exceeds the governed limit. Submit it to the Guard to demonstrate deterministic rejection.".into(),
            reasoning: "The replay intentionally proposes a value outside the governed limit to demonstrate enforcement.".into(),
            proposed_action: ProposedAction {
                action_id: "reduce_speed".into(),
                parameters: json!({"reduction_percent": 45}),
                risk_level: RiskLevel::High,
            },
            sources: vec![KnowledgeCitation {
                title: Some("Governed speed-reduction policy".into()),
                source: Some("local-guard-policy".into()),
                excerpt: Some(
                    "Simulated speed reductions are permitted only within the configured 5-30% range."
                        .into(),
                ),
            }],
            alternatives: vec![],
            created_at: Utc::now(),
        };
        let now = Utc::now();
        let record = IncidentRecord {
            audit: vec![
                audit_event(
                    &package.incident_id,
                    "untrusted_input_detected",
                    "The alarm contained text attempting to override operational policy.",
                    json!({"raw_text": package.alarm.raw_text}),
                ),
                audit_event(
                    &package.incident_id,
                    "unsafe_proposal_staged",
                    "An out-of-policy proposal was staged for Guard validation.",
                    json!({"requested_reduction_percent": 45}),
                ),
            ],
            incident: package,
            status: IncidentStatus::AwaitingApproval,
            proposal: Some(proposal),
            guard_decision: None,
            created_at: now,
            updated_at: now,
        };
        self.store.save(&record)?;
        Ok(vec![record])
    }

    async fn run_offline(&self, packages: Vec<IncidentPackage>) -> Result<Vec<IncidentRecord>> {
        if self.store.factory_connected()? {
            return Err(anyhow!(
                "disconnect the factory cluster before running the network-loss scenario"
            ));
        }
        let mut package = packages
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("network-loss scenario produced no alarm"))?;
        package.connectivity = ConnectivityState::Offline;
        package.context["queued_for_sync"] = json!(true);
        let scenario = self.scenarios.get("single-machine-alarm")?;
        let local_run = self
            .coordinator
            .run(
                &scenario,
                RequestedTarget::Device,
                vec![Message {
                    role: "user".into(),
                    content: incident_prompt(
                        &package,
                        "The factory cluster is unavailable. Assess this alarm locally and provide bounded operator guidance.",
                    ),
                }],
                Some(false),
            )
            .await?;
        let response = local_run
            .fast_response
            .ok_or_else(|| anyhow!("offline scenario returned no local assessment"))?;
        package.local_assessment = Some(LocalAssessment {
            summary: response.content,
            candidate_action_id: Some("request_inspection".into()),
            confidence: response.confidence,
            model_id: response.model_id,
        });
        let now = Utc::now();
        let record = IncidentRecord {
            audit: vec![
                audit_event(
                    &package.incident_id,
                    "factory_connection_lost",
                    "The machine lost connectivity to factory operations.",
                    json!({"connectivity": "offline"}),
                ),
                audit_event(
                    &package.incident_id,
                    "local_assessment_completed",
                    "The machine-local fast agent completed a bounded assessment while offline.",
                    json!({"factory_agent_used": false}),
                ),
                audit_event(
                    &package.incident_id,
                    "incident_queued_for_sync",
                    "The incident was persisted for synchronization after connectivity recovers.",
                    json!({"queued_for_sync": true}),
                ),
            ],
            incident: package,
            status: IncidentStatus::Escalated,
            proposal: None,
            guard_decision: None,
            created_at: now,
            updated_at: now,
        };
        self.store.save(&record)?;
        Ok(vec![record])
    }

    async fn synchronize_incident(&self, mut record: IncidentRecord) -> Result<IncidentRecord> {
        record.incident.connectivity = ConnectivityState::Syncing;
        record.updated_at = Utc::now();
        record.audit.push(audit_event(
            &record.incident.incident_id,
            "synchronization_started",
            "Factory connectivity recovered and the queued incident began synchronization.",
            json!({}),
        ));
        self.store.save(&record)?;

        let factory_scenario = self.scenarios.get("cross-machine-correlation")?;
        let local_summary = record
            .incident
            .local_assessment
            .as_ref()
            .map(|assessment| assessment.summary.clone())
            .unwrap_or_else(|| "No local assessment was recorded.".into());
        let factory_run = self
            .coordinator
            .run(
                &factory_scenario,
                RequestedTarget::Edge,
                vec![
                    Message {
                        role: "user".into(),
                        content: incident_prompt(
                            &record.incident,
                            "This incident was queued while the factory cluster was offline. Reconcile it after recovery and retrieve relevant sources. Return concise operator-facing guidance in at most 180 words. Do not narrate planning or tool calls. Do not emit [cite:N] markers because citations are rendered separately. Propose only a governed action.",
                        ),
                    },
                    Message {
                        role: "assistant".into(),
                        content: format!("Offline machine assessment: {local_summary}"),
                    },
                ],
                Some(false),
            )
            .await?;
        let slow = factory_run
            .slow_response
            .ok_or_else(|| anyhow!("recovery synchronization returned no factory proposal"))?;
        if !slow.success {
            record.incident.connectivity = ConnectivityState::Connected;
            record.audit.push(audit_event(
                &record.incident.incident_id,
                "synchronization_failed",
                "The factory agent could not process the queued incident.",
                json!({"error": slow.error.map(|error| error.message)}),
            ));
            record.updated_at = Utc::now();
            self.store.save(&record)?;
            return Err(anyhow!("factory recovery analysis failed"));
        }
        let sources = slow
            .agentic_evidence
            .as_ref()
            .map(|evidence| evidence.citations.clone())
            .unwrap_or_default();
        let grounded = !sources.is_empty();
        let proposal = ActionProposal {
            proposal_id: Uuid::new_v4().to_string(),
            incident_id: record.incident.incident_id.clone(),
            root_cause_hypothesis:
                "Factory analysis completed after the offline incident synchronized.".into(),
            operator_summary: "The machine handled the incident locally while factory operations were offline. After reconnect, the factory agent retrieved supporting guidance and prepared a bounded 15% speed-reduction proposal for operator review.".into(),
            reasoning: slow.content,
            proposed_action: ProposedAction {
                action_id: "reduce_speed".into(),
                parameters: json!({"reduction_percent": 15}),
                risk_level: RiskLevel::Medium,
            },
            sources,
            alternatives: vec![ProposedAction {
                action_id: "request_inspection".into(),
                parameters: json!({"priority": "urgent"}),
                risk_level: RiskLevel::Low,
            }],
            created_at: Utc::now(),
        };
        record.incident.connectivity = ConnectivityState::Connected;
        record.incident.context["queued_for_sync"] = json!(false);
        record.status = if grounded {
            IncidentStatus::AwaitingApproval
        } else {
            IncidentStatus::Escalated
        };
        record.proposal = Some(proposal.clone());
        record.updated_at = Utc::now();
        record.audit.push(audit_event(
            &record.incident.incident_id,
            if grounded {
                "incident_synchronized"
            } else {
                "synchronization_ungrounded"
            },
            if grounded {
                "The queued incident synchronized exactly once and received a grounded factory proposal."
            } else {
                "The queued incident synchronized, but no sources were returned; operator approval remains unavailable."
            },
            json!({
                "proposal_id": proposal.proposal_id,
                "source_count": proposal.sources.len()
            }),
        ));
        self.store.save(&record)?;
        Ok(record)
    }
}

fn incident_prompt(package: &IncidentPackage, instruction: &str) -> String {
    format!(
        "{instruction}\nIncident package:\n{}",
        serde_json::to_string_pretty(package).unwrap_or_else(|_| "{}".into())
    )
}

fn expert_recommendations(content: &str) -> Vec<String> {
    let recommendations = content
        .lines()
        .map(str::trim)
        .map(|line| {
            line.trim_start_matches(|character: char| {
                character == '-' || character == '*' || character.is_ascii_digit()
            })
            .trim_start_matches(['.', ')', ':', ' '])
            .trim()
        })
        .filter(|line| !line.is_empty())
        .take(3)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if recommendations.is_empty() {
        vec![content.trim().to_string()]
    } else {
        recommendations
    }
}

fn audit_event(
    incident_id: &str,
    event_type: &str,
    summary: &str,
    details: serde_json::Value,
) -> AuditEvent {
    AuditEvent {
        event_id: Uuid::new_v4().to_string(),
        incident_id: incident_id.into(),
        event_type: event_type.into(),
        summary: summary.into(),
        timestamp: Utc::now(),
        details,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ConnectivityState, IncidentPackage, NormalizedAlarm, RiskLevel};

    #[test]
    fn guard_rejects_excessive_speed_reduction() {
        let store = IncidentStore::in_memory().unwrap();
        let incident_id = "incident-1".to_string();
        let proposal = ActionProposal {
            proposal_id: "proposal-1".into(),
            incident_id: incident_id.clone(),
            root_cause_hypothesis: "test".into(),
            operator_summary: "test".into(),
            reasoning: "test".into(),
            proposed_action: ProposedAction {
                action_id: "reduce_speed".into(),
                parameters: json!({"reduction_percent": 45}),
                risk_level: RiskLevel::High,
            },
            sources: vec![],
            alternatives: vec![],
            created_at: Utc::now(),
        };
        let now = Utc::now();
        let record = IncidentRecord {
            incident: IncidentPackage {
                incident_id: incident_id.clone(),
                machine_id: "Robot 17".into(),
                line_id: "Line B".into(),
                vendor_profile: "Contoso Motion Systems".into(),
                machine_model: "CMS-R17".into(),
                firmware_version: "2026.1".into(),
                manual_revision: "M9".into(),
                timestamp: now,
                alarm: NormalizedAlarm {
                    code: "bearing_temperature_critical".into(),
                    raw_code: "CMS.BRG.88".into(),
                    raw_text: "test".into(),
                    severity: "critical".into(),
                },
                context: json!({}),
                local_assessment: None,
                connectivity: ConnectivityState::Connected,
            },
            status: IncidentStatus::AwaitingApproval,
            proposal: Some(proposal),
            guard_decision: None,
            audit: vec![],
            created_at: now,
            updated_at: now,
        };
        store.save(&record).unwrap();
        let guard = GuardService;
        let decision = guard.evaluate(&record, record.proposal.as_ref().unwrap(), "Operator");
        assert!(!decision.permitted);
        assert!(decision.reason.contains("exceeds"));
    }
}
