use anyhow::{Context, Result, bail};
use clap::Parser;
use reqwest::{
    Client,
    header::{HeaderMap, HeaderName, HeaderValue},
    multipart,
};
use serde_json::{Value, json};
use std::{env, path::PathBuf, time::Duration};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "data/eval_dataset.jsonl")]
    dataset: PathBuf,
    #[arg(long, default_value = "factory-maintenance-eval")]
    dataset_name: String,
    #[arg(long, default_value = "factory-maintenance-eval-run")]
    evaluation_name: String,
    #[arg(long)]
    deployment: String,
    #[arg(long)]
    judge_deployment: Option<String>,
    #[arg(long, value_delimiter = ',', default_value = "f1_score,rouge")]
    evaluators: Vec<String>,
    #[arg(long, default_value_t = 2)]
    poll_seconds: u64,
}

fn auth_headers() -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    let Ok(token) = env::var("EDGE_CONTROL_PLANE_TOKEN") else {
        return Ok(headers);
    };
    if env::var("EDGE_CONTROL_PLANE_AUTH_MODE").as_deref() == Ok("bearer") {
        headers.insert(
            reqwest::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))?,
        );
    } else {
        let name = env::var("EDGE_CONTROL_PLANE_AUTH_HEADER")
            .unwrap_or_else(|_| "api-key".into())
            .parse::<HeaderName>()?;
        headers.insert(name, HeaderValue::from_str(&token)?);
    }
    Ok(headers)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let base_url =
        env::var("EDGE_CONTROL_PLANE_URL").context("EDGE_CONTROL_PLANE_URL is required")?;
    let bytes = tokio::fs::read(&args.dataset)
        .await
        .with_context(|| format!("read {}", args.dataset.display()))?;
    let file_name = args
        .dataset
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("eval_dataset.jsonl")
        .to_owned();
    let client = Client::builder()
        .default_headers(auth_headers()?)
        .timeout(Duration::from_secs(60))
        .build()?;
    let form = multipart::Form::new()
        .text("name", args.dataset_name.clone())
        .text("format", "jsonl")
        .part(
            "file",
            multipart::Part::bytes(bytes)
                .file_name(file_name)
                .mime_str("application/jsonl")?,
        );
    let upload = client
        .post(format!("{base_url}/api/v1/datasets"))
        .multipart(form)
        .send()
        .await?;
    if !upload.status().is_success() && upload.status().as_u16() != 409 {
        bail!("dataset upload failed: HTTP {}", upload.status());
    }

    let mut payload = json!({
        "name": args.evaluation_name,
        "datasetRef": args.dataset_name,
        "modelRef": args.deployment,
        "evaluators": args.evaluators
    });
    if let Some(judge) = args.judge_deployment {
        payload["judgeModelRef"] = json!(judge);
    }
    let created = client
        .post(format!("{base_url}/api/v1/evaluations"))
        .json(&payload)
        .send()
        .await?;
    if !created.status().is_success() && created.status().as_u16() != 409 {
        bail!("evaluation submission failed: HTTP {}", created.status());
    }

    loop {
        let body: Value = client
            .get(format!(
                "{base_url}/api/v1/evaluations/{}",
                args.evaluation_name
            ))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        println!("{}", serde_json::to_string_pretty(&body)?);
        match body["phase"].as_str() {
            Some("Succeeded") => return Ok(()),
            Some("Failed" | "Error") => bail!("evaluation failed"),
            _ => tokio::time::sleep(Duration::from_secs(args.poll_seconds)).await,
        }
    }
}
