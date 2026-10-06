use crate::{
    config::DemoSettings,
    domain::{
        ApproveActionRequest, ChatApiRequest, CreateReduceSpeedRequest, IncidentApprovalRequest,
        ManagementBriefGenerateRequest, ManagementBriefPreviewRequest, Message, PreflightResult,
        TargetType,
    },
    providers::ProviderRegistry,
    services::{ActionService, FastSlowCoordinator, GovernedFloorService, ScenarioService},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use futures::future::join_all;
use serde_json::{Value, json};
use std::sync::Arc;
use tower_http::{services::ServeDir, trace::TraceLayer};

#[derive(Clone)]
pub struct AppState {
    pub settings: Arc<DemoSettings>,
    pub registry: Arc<ProviderRegistry>,
    pub scenarios: ScenarioService,
    pub coordinator: FastSlowCoordinator,
    pub actions: ActionService,
    pub governed_floor: GovernedFloorService,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(machine_hmi))
        .route("/machine", get(machine_hmi))
        .route("/operations", get(operations_view))
        .route("/management", get(management_view))
        .route("/demo", get(demo_view))
        .route("/api/status", get(application_status))
        .route("/api/scenarios", get(list_scenarios))
        .route("/api/providers", get(list_providers))
        .route("/api/providers/{provider_id}/health", get(provider_health))
        .route(
            "/api/providers/{provider_id}/capabilities",
            get(provider_capabilities),
        )
        .route("/api/preflight", post(preflight))
        .route("/api/chat", post(chat))
        .route(
            "/api/actions/reduce-speed/requests",
            post(create_reduce_speed_request),
        )
        .route("/api/actions/{action_id}/approve", post(approve_action))
        .route("/api/evaluations", post(create_evaluation))
        .route("/api/incidents", get(list_incidents))
        .route("/api/incidents/{incident_id}", get(get_incident))
        .route(
            "/api/incidents/{incident_id}/approve",
            post(approve_incident),
        )
        .route("/api/incidents/{incident_id}/reject", post(reject_incident))
        .route(
            "/api/demo/scenarios/{scenario_id}/run",
            post(run_guided_scenario),
        )
        .route(
            "/api/demo/management-brief/preview",
            post(preview_management_brief),
        )
        .route(
            "/api/demo/management-brief/generate",
            post(create_management_brief),
        )
        .route("/api/demo/connectivity", get(get_factory_connectivity))
        .route(
            "/api/demo/connectivity/{state}",
            post(set_factory_connectivity),
        )
        .route("/api/demo/reset", post(reset_guided_demo))
        .nest_service("/static", ServeDir::new("app/ui/static"))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn machine_hmi() -> Html<&'static str> {
    Html(include_str!("../app/ui/static/machine.html"))
}

async fn operations_view() -> Html<&'static str> {
    Html(include_str!("../app/ui/static/operations.html"))
}

async fn management_view() -> Html<&'static str> {
    Html(include_str!("../app/ui/static/management.html"))
}

async fn demo_view() -> Html<&'static str> {
    Html(include_str!("../app/ui/static/index.html"))
}

async fn application_status() -> Json<Value> {
    Json(json!({
        "name": "Governed Floor",
        "version": env!("CARGO_PKG_VERSION"),
        "runtime": "rust",
        "operator_approved_actions": true,
        "action_connector": "simulated",
        "synthetic_data": true,
        "preview_notice": "Foundry Local on Azure Local capabilities are preview and environment-specific."
    }))
}

async fn list_scenarios(State(state): State<AppState>) -> Json<Value> {
    Json(serde_json::to_value(state.scenarios.list()).expect("serialize scenarios"))
}

async fn list_providers(State(state): State<AppState>) -> Json<Value> {
    let providers = state
        .registry
        .all()
        .into_iter()
        .map(|provider| {
            let settings = state.settings.providers.get(provider.id());
            let enabled = settings.is_some_and(|settings| settings.enabled);
            let configured = settings.is_some_and(|settings| settings.configured());
            json!({
                "provider_id": provider.id(),
                "target_type": provider.target_type(),
                "capabilities": provider.capabilities(),
                "enabled": enabled,
                "configured": configured,
                "mock": provider.is_mock()
            })
        })
        .collect::<Vec<_>>();
    Json(json!(providers))
}

async fn provider_health(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let provider = state
        .registry
        .by_id(&provider_id)
        .ok_or_else(|| AppError::not_found("unknown provider"))?;
    Ok(Json(json!(provider.health().await)))
}

async fn provider_capabilities(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let provider = state
        .registry
        .by_id(&provider_id)
        .ok_or_else(|| AppError::not_found("unknown provider"))?;
    Ok(Json(json!(provider.capabilities())))
}

async fn preflight(State(state): State<AppState>) -> Json<PreflightResult> {
    let statuses = join_all(
        state
            .registry
            .all()
            .into_iter()
            .map(|provider| async move { provider.health().await }),
    )
    .await;
    let expected = statuses
        .iter()
        .filter(|status| {
            state
                .settings
                .providers
                .get(&status.provider_id)
                .is_some_and(|settings| settings.enabled && settings.provider_type != "mock")
        })
        .collect::<Vec<_>>();
    let ready = !expected.is_empty()
        && expected
            .iter()
            .all(|status| status.configured && status.reachable);
    Json(PreflightResult {
        providers: statuses,
        ready,
        message: if ready {
            "All enabled providers are configured and reachable".into()
        } else {
            "One or more enabled providers are not configured or reachable".into()
        },
    })
}

async fn chat(
    State(state): State<AppState>,
    Json(request): Json<ChatApiRequest>,
) -> Result<Json<Value>, AppError> {
    let scenario = state.scenarios.get(&request.scenario_id)?;
    let messages = request.messages.unwrap_or_else(|| {
        vec![Message {
            role: "user".into(),
            content: scenario.default_prompt.clone(),
        }]
    });
    let result = state
        .coordinator
        .run(&scenario, request.target, messages, request.allow_fallback)
        .await?;
    state.actions.register_run(&scenario, &result).await;
    Ok(Json(json!(result)))
}

async fn create_reduce_speed_request(
    State(state): State<AppState>,
    Json(request): Json<CreateReduceSpeedRequest>,
) -> Result<Json<Value>, AppError> {
    let action = state.actions.create_reduce_speed(request).await?;
    Ok(Json(json!(action)))
}

async fn approve_action(
    State(state): State<AppState>,
    Path(action_id): Path<String>,
    Json(request): Json<ApproveActionRequest>,
) -> Result<Json<Value>, AppError> {
    let action = state
        .actions
        .approve(&action_id, request.approved, request.operator_id)
        .await?;
    Ok(Json(json!(action)))
}

async fn create_evaluation(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let edge = state.registry.for_target(TargetType::Edge)?;
    if !edge.capabilities().local_evaluation {
        return Err(AppError::new(
            StatusCode::NOT_IMPLEMENTED,
            "Local evaluation is unavailable for the configured edge provider",
        ));
    }
    Err(AppError::new(
        StatusCode::NOT_IMPLEMENTED,
        "Evaluation capability is declared but no verified control-plane URL is configured. Use the factory-eval binary after setting EDGE_CONTROL_PLANE_URL.",
    ))
}

async fn list_incidents(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(state.governed_floor.list()?)))
}

async fn get_incident(
    State(state): State<AppState>,
    Path(incident_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(state.governed_floor.get(&incident_id)?)))
}

async fn run_guided_scenario(
    State(state): State<AppState>,
    Path(scenario_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(
        state.governed_floor.run_demo(&scenario_id).await?
    )))
}

async fn preview_management_brief(
    State(state): State<AppState>,
    Json(request): Json<ManagementBriefPreviewRequest>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        state.governed_floor.preview_management_brief(request)?,
    ))
}

async fn create_management_brief(
    State(state): State<AppState>,
    Json(request): Json<ManagementBriefGenerateRequest>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        state
            .governed_floor
            .create_management_brief(&request.preview_id)
            .await?,
    ))
}

async fn reset_guided_demo(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    state.governed_floor.reset()?;
    Ok(Json(json!({"reset": true})))
}

async fn get_factory_connectivity(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    Ok(Json(json!({
        "connected": state.governed_floor.factory_connected()?
    })))
}

async fn set_factory_connectivity(
    State(state): State<AppState>,
    Path(connectivity): Path<String>,
) -> Result<Json<Value>, AppError> {
    let connected = match connectivity.as_str() {
        "connected" => true,
        "offline" => false,
        _ => {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                "connectivity must be 'connected' or 'offline'",
            ));
        }
    };
    let synchronized = state
        .governed_floor
        .set_factory_connected(connected)
        .await?;
    Ok(Json(json!({
        "connected": connected,
        "synchronized": synchronized
    })))
}

async fn approve_incident(
    State(state): State<AppState>,
    Path(incident_id): Path<String>,
    Json(request): Json<IncidentApprovalRequest>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(
        state
            .governed_floor
            .approve(&incident_id, &request.operator_id)?
    )))
}

async fn reject_incident(
    State(state): State<AppState>,
    Path(incident_id): Path<String>,
    Json(request): Json<IncidentApprovalRequest>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(
        state
            .governed_floor
            .reject(&incident_id, &request.operator_id)?
    )))
}

pub struct AppError {
    status: StatusCode,
    message: String,
}

impl AppError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }
}

impl From<anyhow::Error> for AppError {
    fn from(error: anyhow::Error) -> Self {
        Self::new(StatusCode::BAD_REQUEST, error.to_string())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "detail": self.message }))).into_response()
    }
}
