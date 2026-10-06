use anyhow::{Context, Result, bail};
use clap::Parser;
use reqwest::Client;
use serde_json::{Value, json};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:8000")]
    base_url: String,
    #[arg(long, default_value = "auto")]
    target: String,
    #[arg(long, default_value = "robot-bearing-alarm")]
    scenario: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let client = Client::new();
    client
        .get(format!("{}/api/status", args.base_url))
        .send()
        .await?
        .error_for_status()?;
    let body: Value = client
        .post(format!("{}/api/chat", args.base_url))
        .json(&json!({
            "scenario_id": args.scenario,
            "target": args.target
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let evidence = ["fast_response", "slow_response"]
        .into_iter()
        .filter_map(|name| body.get(name).filter(|value| !value.is_null()))
        .collect::<Vec<_>>();
    if evidence.is_empty()
        || evidence
            .iter()
            .any(|item| item.get("request_id").and_then(Value::as_str).is_none())
    {
        bail!("normalized response evidence is missing");
    }
    let mode = body["mode"].as_str().context("missing run mode")?;
    let targets = evidence
        .iter()
        .map(|item| {
            format!(
                "{}:{}",
                item["target_type"].as_str().unwrap_or("?"),
                item["provider_id"].as_str().unwrap_or("?")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    println!("Smoke test passed: {mode} using {targets}");
    Ok(())
}
