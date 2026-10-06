use anyhow::Result;
use clap::Parser;
use reqwest::Client;
use serde_json::{Value, json};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:8000")]
    base_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let client = Client::new();
    let preflight: Value = client
        .post(format!("{}/api/preflight", args.base_url))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    println!(
        "Preflight: {}",
        preflight["message"].as_str().unwrap_or("unknown")
    );
    let steps = [
        ("single-machine-alarm", "device"),
        ("robot-bearing-alarm", "auto"),
        ("cross-machine-correlation", "edge"),
        ("management-summary", "cloud"),
    ];
    for (scenario, target) in steps {
        let body: Value = client
            .post(format!("{}/api/chat", args.base_url))
            .json(&json!({"scenario_id": scenario, "target": target}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let evidence = if body["slow_response"].is_null() {
            &body["fast_response"]
        } else {
            &body["slow_response"]
        };
        println!(
            "{}: {} -> {} / {} / {} ms",
            scenario,
            body["mode"].as_str().unwrap_or("?"),
            evidence["target_type"].as_str().unwrap_or("?"),
            evidence["provider_id"].as_str().unwrap_or("?"),
            evidence["latency_ms"].as_u64().unwrap_or_default()
        );
    }
    Ok(())
}
