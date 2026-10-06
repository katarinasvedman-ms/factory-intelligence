use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetType {
    Device,
    Edge,
    Cloud,
    Mock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequestedTarget {
    Device,
    Edge,
    Cloud,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataClassification {
    LocalOnly,
    CloudAllowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuidanceStatus {
    Current,
    Stale,
    RevalidationRequired,
    Superseded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub request_id: String,
    pub scenario_id: String,
    pub messages: Vec<Message>,
    pub temperature: Option<f64>,
    pub max_output_tokens: Option<u32>,
    pub stream: bool,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub request_id: String,
    pub provider_id: String,
    pub target_type: TargetType,
    pub model_id: String,
    pub endpoint_alias: String,
    #[serde(default)]
    pub content: String,
    pub finish_reason: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub latency_ms: u64,
    pub fallback_used: bool,
    pub success: bool,
    pub timestamp: DateTime<Utc>,
    pub confidence: Option<f64>,
    pub raw_response_logged: bool,
    pub mock: bool,
    #[serde(default)]
    pub agentic_evidence: Option<AgenticRunEvidence>,
    pub error: Option<ProviderError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeCitation {
    pub title: Option<String>,
    pub source: Option<String>,
    pub excerpt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgenticStepEvidence {
    pub step_type: String,
    pub tool_name: Option<String>,
    pub details: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgenticRunEvidence {
    pub thread_id: String,
    pub run_id: String,
    pub agent_id: String,
    pub status: String,
    pub steps: Vec<AgenticStepEvidence>,
    pub citations: Vec<KnowledgeCitation>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderCapabilities {
    pub streaming: bool,
    pub tool_calling: bool,
    pub structured_output: bool,
    pub image_input: bool,
    pub local_evaluation: bool,
    pub disconnected_inference: bool,
    pub shared_endpoint: bool,
    pub model_management: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub provider_id: String,
    pub configured: bool,
    pub reachable: bool,
    pub authenticated: Option<bool>,
    pub model_available: Option<bool>,
    pub last_check_timestamp: DateTime<Utc>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineEvent {
    pub machine_id: String,
    pub line_id: String,
    pub event_timestamp: DateTime<Utc>,
    pub alarm_code: String,
    pub vibration_level: Value,
    pub bearing_temperature_c: f64,
    pub motor_current_a: Option<f64>,
    pub operating_state: String,
    #[serde(default)]
    pub maintenance_notes: Vec<String>,
    pub data_classification: DataClassification,
    pub severity: String,
    pub evidence_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceSnapshot {
    pub snapshot_id: String,
    pub created_at: DateTime<Utc>,
    pub machine_event: MachineEvent,
    pub local_result: ChatResponse,
    pub peer_machine_summary: Option<Value>,
    pub evidence_version: String,
    pub data_classification: DataClassification,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationDecision {
    pub escalate: bool,
    pub target: Option<TargetType>,
    pub rule_id: String,
    pub reason: String,
    pub snapshot_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub id: String,
    pub title: String,
    pub description: String,
    pub default_prompt: String,
    pub recommended_target: RequestedTarget,
    pub data_classification: DataClassification,
    #[serde(default)]
    pub requires_fast_slow: bool,
    #[serde(default)]
    pub requires_shared_context: bool,
    pub event_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatApiRequest {
    pub scenario_id: String,
    pub target: RequestedTarget,
    pub messages: Option<Vec<Message>>,
    pub allow_fallback: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    pub requested_target: RequestedTarget,
    pub selected_target: TargetType,
    pub rule_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatedRun {
    pub mode: String,
    pub routing: RoutingDecision,
    pub escalation: Option<EscalationDecision>,
    pub snapshot: Option<EvidenceSnapshot>,
    pub fast_response: Option<ChatResponse>,
    pub slow_response: Option<ChatResponse>,
    pub guidance_status: Option<GuidanceStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreflightResult {
    pub providers: Vec<ProviderStatus>,
    pub ready: bool,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    PendingApproval,
    Executed,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateReduceSpeedRequest {
    pub scenario_id: String,
    pub inference_request_id: String,
    pub reduction_percent: u8,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApproveActionRequest {
    pub approved: bool,
    pub operator_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReduceSpeedAction {
    pub action_id: String,
    pub scenario_id: String,
    pub inference_request_id: String,
    pub machine_id: String,
    pub reduction_percent: u8,
    pub connector_id: String,
    pub simulated: bool,
    pub status: ActionStatus,
    pub requested_at: DateTime<Utc>,
    pub approved_at: Option<DateTime<Utc>>,
    pub executed_at: Option<DateTime<Utc>>,
    pub approved_by: Option<String>,
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncidentStatus {
    Detected,
    LocallyAssessed,
    Escalated,
    Correlating,
    AwaitingApproval,
    Approved,
    Rejected,
    Executed,
    Monitoring,
    Failed,
    Resolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectivityState {
    Connected,
    Offline,
    Syncing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VendorAlarm {
    pub vendor_profile: String,
    pub machine_id: String,
    pub line_id: String,
    pub machine_model: String,
    pub firmware_version: String,
    pub manual_revision: String,
    pub timestamp: DateTime<Utc>,
    pub code: String,
    pub raw_text: String,
    pub severity: String,
    pub signals: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedAlarm {
    pub code: String,
    pub raw_code: String,
    pub raw_text: String,
    pub severity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAssessment {
    pub summary: String,
    pub candidate_action_id: Option<String>,
    pub confidence: Option<f64>,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncidentPackage {
    pub incident_id: String,
    pub machine_id: String,
    pub line_id: String,
    pub vendor_profile: String,
    pub machine_model: String,
    pub firmware_version: String,
    pub manual_revision: String,
    pub timestamp: DateTime<Utc>,
    pub alarm: NormalizedAlarm,
    pub context: Value,
    pub local_assessment: Option<LocalAssessment>,
    pub connectivity: ConnectivityState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposedAction {
    pub action_id: String,
    pub parameters: Value,
    pub risk_level: RiskLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionProposal {
    pub proposal_id: String,
    pub incident_id: String,
    pub root_cause_hypothesis: String,
    #[serde(default)]
    pub operator_summary: String,
    pub reasoning: String,
    pub proposed_action: ProposedAction,
    pub sources: Vec<KnowledgeCitation>,
    #[serde(default)]
    pub alternatives: Vec<ProposedAction>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardDecision {
    pub decision_id: String,
    pub proposal_id: String,
    pub incident_id: String,
    pub permitted: bool,
    pub reason: String,
    pub checked_at: DateTime<Utc>,
    pub approved_by: Option<String>,
    pub executed: bool,
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event_id: String,
    pub incident_id: String,
    pub event_type: String,
    pub summary: String,
    pub timestamp: DateTime<Utc>,
    pub details: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncidentRecord {
    pub incident: IncidentPackage,
    pub status: IncidentStatus,
    pub proposal: Option<ActionProposal>,
    pub guard_decision: Option<GuardDecision>,
    pub audit: Vec<AuditEvent>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IncidentApprovalRequest {
    pub operator_id: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagementBriefMode {
    Consolidated,
    LatestExecuted,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManagementBriefPreviewRequest {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub mode: ManagementBriefMode,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManagementBriefGenerateRequest {
    pub preview_id: String,
}
