use anyhow::{Result, bail};
use clap::Parser;
use reqwest::Client;
use serde_json::{Value, json};
use std::{collections::HashSet, time::Instant};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:8000")]
    base_url: String,
}

async fn invoke(client: Client, base_url: String, logical_client: &'static str) -> Result<Value> {
    let started = Instant::now();
    let body: Value = client
        .post(format!("{base_url}/api/chat"))
        .json(&json!({
            "scenario_id": "cross-machine-correlation",
            "target": "edge",
            "messages": [{
                "role": "user",
                "content": format!(
                    "Logical client {logical_client}: correlate the synthetic Line A events and provide advisory investigation priorities."
                )
            }]
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let evidence = &body["slow_response"];
    Ok(json!({
        "logical_client": logical_client,
        "request_id": evidence["request_id"],
        "provider_id": evidence["provider_id"],
        "endpoint_alias": evidence["endpoint_alias"],
        "success": evidence["success"],
        "application_latency_ms": started.elapsed().as_millis(),
        "provider_latency_ms": evidence["latency_ms"]
    }))
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let client = Client::new();
    let (first, second) = tokio::try_join!(
        invoke(client.clone(), args.base_url.clone(), "machine-client-a"),
        invoke(client, args.base_url, "maintenance-app-b")
    )?;
    println!("{}", serde_json::to_string_pretty(&first)?);
    println!("{}", serde_json::to_string_pretty(&second)?);
    let results = [&first, &second];
    let endpoints = results
        .iter()
        .filter_map(|item| item["endpoint_alias"].as_str())
        .collect::<HashSet<_>>();
    let requests = results
        .iter()
        .filter_map(|item| item["request_id"].as_str())
        .collect::<HashSet<_>>();
    if endpoints.len() != 1
        || requests.len() != 2
        || results
            .iter()
            .any(|item| item["success"].as_bool() != Some(true))
    {
        bail!("concurrent edge validation failed");
    }
    Ok(())
}
