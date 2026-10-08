use anyhow::{Context, Result};
use factory_intelligence::{
    config::load_settings,
    providers::ProviderRegistry,
    services::{
        AdvisoryProcessor, AdvisorySettings, FastSlowCoordinator, InferenceService,
        MqttAdvisoryWorker, RoutingPolicy, ScenarioService, WorkerJobStore,
    },
};
use std::{env, path::PathBuf, sync::Arc};
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
    let inference = InferenceService::new(settings.application, registry);
    let coordinator = FastSlowCoordinator::new(scenarios.clone(), RoutingPolicy, inference);
    let processor = AdvisoryProcessor::new(scenarios, coordinator);
    let mqtt = AdvisorySettings::worker_from_env()?;
    let store_path = env::var("ADVISORY_WORKER_DB_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("data/advisory-worker.db"));
    let jobs = WorkerJobStore::open(&store_path)
        .with_context(|| format!("open advisory worker store {}", store_path.display()))?;
    MqttAdvisoryWorker::new(processor, mqtt, jobs).run().await
}
