use anyhow::{Result, anyhow, ensure};
use chrono::{Duration, Utc};
use clap::Parser;
use reqwest::Client;
use serde_json::{Value, json};
use tokio::time::{Duration as TokioDuration, sleep};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "http://127.0.0.1:8000")]
    base_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let client = Client::new();
    client
        .post(format!("{}/api/demo/reset", args.base_url))
        .send()
        .await?
        .error_for_status()?;

    let unsafe_records = run_scenario(&client, &args.base_url, "unsafe-action").await?;
    let unsafe_incident = unsafe_records
        .first()
        .ok_or_else(|| anyhow!("unsafe scenario returned no incident"))?;
    let rejected = decide(
        &client,
        &args.base_url,
        unsafe_incident["incident"]["incident_id"]
            .as_str()
            .ok_or_else(|| anyhow!("unsafe incident has no id"))?,
        "approve",
        "factory_operations",
    )
    .await?;
    ensure!(
        rejected["status"] == "rejected" && rejected["guard_decision"]["permitted"] == false,
        "Guard did not reject the unsafe action"
    );
    println!("PASS governance rejection");

    let routine_records = run_scenario(&client, &args.base_url, "routine-local").await?;
    let routine = routine_records
        .first()
        .ok_or_else(|| anyhow!("routine scenario returned no incident"))?;
    ensure!(
        routine["status"] == "correlating" || routine["status"] == "awaiting_approval",
        "Routine alarm did not request a factory advisory"
    );
    let routine_id = routine["incident"]["incident_id"]
        .as_str()
        .ok_or_else(|| anyhow!("routine incident has no id"))?;
    let advised = wait_for_advisory(&client, &args.base_url, routine_id).await?;
    ensure!(
        advised["status"] == "awaiting_approval"
            && advised["proposal"]["sources"]
                .as_array()
                .is_some_and(|sources| !sources.is_empty()),
        "Routine incident did not receive a grounded advisory"
    );
    let routine_approved = decide(
        &client,
        &args.base_url,
        routine_id,
        "approve",
        "machine_hmi",
    )
    .await?;
    ensure!(
        routine_approved["status"] == "executed"
            && routine_approved["audit"]
                .as_array()
                .and_then(|events| events.last())
                .is_some_and(|event| event["details"]["decision_source"] == "machine_hmi"),
        "Machine-HMI approval did not execute through the shared Guard"
    );
    println!("PASS machine advisory and HMI approval");

    set_connectivity(&client, &args.base_url, "offline").await?;
    let offline_records = run_scenario(&client, &args.base_url, "network-loss").await?;
    let queued = offline_records
        .first()
        .ok_or_else(|| anyhow!("network-loss scenario returned no incident"))?;
    ensure!(
        queued["incident"]["connectivity"] == "offline"
            && queued["incident"]["context"]["queued_for_sync"] == true,
        "Offline incident was not queued for synchronization"
    );
    let recovery = set_connectivity(&client, &args.base_url, "connected").await?;
    let synchronized = recovery["synchronized"]
        .as_array()
        .ok_or_else(|| anyhow!("recovery returned no synchronized incident list"))?;
    ensure!(
        synchronized.len() == 1
            && synchronized[0]["status"] == "awaiting_approval"
            && synchronized[0]["proposal"]["sources"]
                .as_array()
                .is_some_and(|sources| !sources.is_empty()),
        "Queued incident did not receive one grounded recovery proposal"
    );
    let second_recovery = set_connectivity(&client, &args.base_url, "connected").await?;
    ensure!(
        second_recovery["synchronized"]
            .as_array()
            .is_some_and(Vec::is_empty),
        "A second reconnect duplicated incident synchronization"
    );
    println!("PASS network loss and recovery");

    let cascade_records = run_scenario(&client, &args.base_url, "cross-vendor-cascade").await?;
    let primary = cascade_records
        .iter()
        .find(|record| !record["proposal"].is_null())
        .ok_or_else(|| anyhow!("cascade returned no action proposal"))?;
    ensure!(
        primary["proposal"]["sources"]
            .as_array()
            .is_some_and(|sources| !sources.is_empty()),
        "Cascade proposal returned no citations"
    );
    let approved = decide(
        &client,
        &args.base_url,
        primary["incident"]["incident_id"]
            .as_str()
            .ok_or_else(|| anyhow!("cascade incident has no id"))?,
        "approve",
        "factory_operations",
    )
    .await?;
    ensure!(
        approved["status"] == "executed" && approved["guard_decision"]["permitted"] == true,
        "Guard did not execute the bounded cascade action"
    );
    let incidents: Vec<Value> = client
        .get(format!("{}/api/incidents", args.base_url))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        incidents.iter().any(|record| {
            record["incident"]["machine_id"] == "Packer 12" && record["status"] == "monitoring"
        }),
        "Downstream packer did not transition to monitoring"
    );
    println!("PASS cross-vendor cascade");

    let report_to = Utc::now();
    let preview: Value = client
        .post(format!(
            "{}/api/demo/management-brief/preview",
            args.base_url
        ))
        .json(&json!({
            "from": report_to - Duration::hours(24),
            "to": report_to,
            "mode": "consolidated"
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        preview["incident_count"]
            .as_u64()
            .is_some_and(|count| count >= 2)
            && preview["payload"]["classification"] == "cloud_allowed"
            && preview["policy"]["machine_authority"] == false,
        "Management preview is missing the approved scoped payload"
    );
    let brief: Value = client
        .post(format!(
            "{}/api/demo/management-brief/generate",
            args.base_url
        ))
        .json(&json!({"preview_id": preview["preview_id"]}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    ensure!(
        brief["expert_recommendations"]
            .as_array()
            .is_some_and(|recommendations| !recommendations.is_empty()),
        "Management brief did not contain Expert recommendations"
    );
    ensure!(
        brief["governance"]["classification"] == "cloud_allowed"
            && brief["governance"]["machine_authority"] == false,
        "Management brief governance envelope is invalid"
    );
    println!("PASS approved management brief");
    println!("Governed Floor rehearsal passed.");
    Ok(())
}

async fn set_connectivity(client: &Client, base_url: &str, state: &str) -> Result<Value> {
    Ok(client
        .post(format!("{base_url}/api/demo/connectivity/{state}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

async fn run_scenario(client: &Client, base_url: &str, scenario_id: &str) -> Result<Vec<Value>> {
    Ok(client
        .post(format!("{base_url}/api/demo/scenarios/{scenario_id}/run"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

async fn decide(
    client: &Client,
    base_url: &str,
    incident_id: &str,
    decision: &str,
    source: &str,
) -> Result<Value> {
    Ok(client
        .post(format!("{base_url}/api/incidents/{incident_id}/{decision}"))
        .json(&json!({
            "operator_id": "Rehearsal Operator",
            "decision_source": source
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

async fn wait_for_advisory(client: &Client, base_url: &str, incident_id: &str) -> Result<Value> {
    for _ in 0..90 {
        let record: Value = client
            .get(format!("{base_url}/api/incidents/{incident_id}"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if record["status"] != "correlating" {
            return Ok(record);
        }
        sleep(TokioDuration::from_secs(2)).await;
    }
    Err(anyhow!(
        "routine advisory did not complete within 180 seconds"
    ))
}
