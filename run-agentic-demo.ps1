[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$DomainName,

    [Parameter(Mandatory)]
    [string]$ClientId,

    [Parameter(Mandatory)]
    [string]$AgentId
)

$ErrorActionPreference = "Stop"

$token = az account get-access-token `
    --resource "api://$ClientId" `
    --query accessToken `
    --output tsv
if ($LASTEXITCODE -ne 0 -or -not $token) {
    throw "Failed to acquire the Agentic Retrieval access token."
}

$env:EDGE_AGENTIC_BASE_URL = "https://$DomainName"
$env:EDGE_AGENTIC_TOKEN = $token
$env:EDGE_AGENT_ID = $AgentId
$env:DEMO_CONFIG = "config\demo.agentic.yaml"

cargo run --release --bin factory-intelligence
