[CmdletBinding()]
param(
    [string]$AgenticDomain = $env:FACTORY_AGENTIC_DOMAIN,
    [string]$ClientId = $env:FACTORY_AGENTIC_CLIENT_ID,
    [string]$FoundryBaseUrl = $env:FOUNDRY_BASE_URL,
    [string]$FoundryExpertModel = $env:FOUNDRY_EXPERT_MODEL,
    [string]$ConfigPath = "config\demo.aks-agentic.yaml"
)

$ErrorActionPreference = "Stop"

$required = @{
    AgenticDomain = $AgenticDomain
    ClientId = $ClientId
    FoundryBaseUrl = $FoundryBaseUrl
    FoundryExpertModel = $FoundryExpertModel
}
foreach ($entry in $required.GetEnumerator()) {
    if ([string]::IsNullOrWhiteSpace($entry.Value)) {
        throw "$($entry.Key) is required. Pass it as a parameter or configure the corresponding environment variable."
    }
}

$edgeToken = az account get-access-token `
    --resource "api://$ClientId" `
    --query accessToken `
    --output tsv
if (-not $edgeToken) {
    throw "Unable to acquire the Agentic Retrieval token. Run az login and try again."
}

$foundryToken = az account get-access-token `
    --scope "https://cognitiveservices.azure.com/.default" `
    --query accessToken `
    --output tsv
if (-not $foundryToken) {
    throw "Unable to acquire the Foundry Expert token."
}

$baseUrl = "https://$AgenticDomain"
$knowledgeBases = Invoke-RestMethod `
    -Uri "$baseUrl/knowledge-bases?limit=1" `
    -Headers @{ Authorization = "Bearer $edgeToken" } `
    -TimeoutSec 60
$agentId = @($knowledgeBases.data)[0].id
if (-not $agentId) {
    throw "Agentic Retrieval did not return a default knowledge base."
}

$env:EDGE_AGENTIC_BASE_URL = $baseUrl
$env:EDGE_AGENTIC_TOKEN = $edgeToken
$env:EDGE_AGENT_ID = $agentId
$env:FOUNDRY_BASE_URL = $FoundryBaseUrl
$env:FOUNDRY_EXPERT_MODEL = $FoundryExpertModel
$env:FOUNDRY_AUTHORIZATION = "Bearer $foundryToken"
$env:DEMO_CONFIG = $ConfigPath

Write-Host "Starting Factory Intelligence with:"
Write-Host "  Fast:   local Foundry Local model"
Write-Host "  Slow:   AKS Agentic Retrieval + authenticated MCP manuals + Foundry GPT-5 mini"
Write-Host "  Expert: Foundry GPT-5.4"

cargo run --release --bin factory-intelligence
