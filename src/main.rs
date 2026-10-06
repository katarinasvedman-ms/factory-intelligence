use anyhow::{Context, Result};
use factory_intelligence::{
    api::{AppState, router},
    config::load_settings,
    providers::ProviderRegistry,
    services::{
        ActionService, FastSlowCoordinator, GovernedFloorService, IncidentStore, InferenceService,
        RoutingPolicy, ScenarioService,
    },
};
use std::{env, sync::Arc};
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let settings = load_settings()?;
    let registry = Arc::new(ProviderRegistry::new(&settings)?);
    let scenarios = ScenarioService::load()?;
    let inference = InferenceService::new(settings.application.clone(), registry.clone());
    let coordinator = FastSlowCoordinator::new(scenarios.clone(), RoutingPolicy, inference);
    let governed_floor = GovernedFloorService::new(
        IncidentStore::open_default()?,
        scenarios.clone(),
        coordinator.clone(),
    );
    let state = AppState {
        settings: Arc::new(settings),
        registry,
        scenarios,
        coordinator,
        actions: ActionService::default(),
        governed_floor,
    };
    let address = env::var("BIND_ADDRESS").unwrap_or_else(|_| "127.0.0.1:8000".into());
    let listener = TcpListener::bind(&address)
        .await
        .with_context(|| format!("failed to bind {address}"))?;
    info!(event = "server_started", address);
    axum::serve(listener, router(state))
        .await
        .context("server failed")
}
