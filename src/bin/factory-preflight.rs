use anyhow::Result;
use factory_intelligence::{config::load_settings, providers::ProviderRegistry};
use futures::future::join_all;

#[tokio::main]
async fn main() -> Result<()> {
    let settings = load_settings()?;
    let registry = ProviderRegistry::new(&settings)?;
    let statuses = join_all(
        registry
            .all()
            .into_iter()
            .map(|provider| async move { provider.health().await }),
    )
    .await;
    println!("{}", serde_json::to_string_pretty(&statuses)?);
    let configured = statuses
        .iter()
        .filter(|status| status.configured)
        .collect::<Vec<_>>();
    if configured.is_empty() || configured.iter().any(|status| !status.reachable) {
        std::process::exit(1);
    }
    Ok(())
}
