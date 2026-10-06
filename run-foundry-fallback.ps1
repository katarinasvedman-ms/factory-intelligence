[CmdletBinding()]
param(
    [string]$FoundryBaseUrl = $env:FOUNDRY_BASE_URL,
    [string]$FoundrySlowModel = $env:FOUNDRY_SLOW_MODEL,
    [string]$FoundryExpertModel = $env:FOUNDRY_EXPERT_MODEL,
    [string]$ConfigPath = "config\demo.foundry.yaml"
)

$ErrorActionPreference = "Stop"

foreach ($entry in @{
    FoundryBaseUrl = $FoundryBaseUrl
    FoundrySlowModel = $FoundrySlowModel
    FoundryExpertModel = $FoundryExpertModel
}.GetEnumerator()) {
    if ([string]::IsNullOrWhiteSpace($entry.Value)) {
        throw "$($entry.Key) is required. Pass it as a parameter or environment variable."
    }
}

$token = az account get-access-token `
    --scope "https://cognitiveservices.azure.com/.default" `
    --query accessToken `
    --output tsv

if (-not $token) {
    throw "Unable to acquire a Microsoft Foundry access token. Run az login and try again."
}

$env:FOUNDRY_AUTHORIZATION = "Bearer $token"
$env:DEMO_CONFIG = $ConfigPath
$env:FOUNDRY_BASE_URL = $FoundryBaseUrl
$env:FOUNDRY_SLOW_MODEL = $FoundrySlowModel
$env:FOUNDRY_EXPERT_MODEL = $FoundryExpertModel

Write-Host "Starting Factory Intelligence with:"
Write-Host "  Fast:   local Foundry model"
Write-Host "  Slow:   $FoundrySlowModel with application-side manual retrieval"
Write-Host "  Expert: $FoundryExpertModel"

cargo run --release --bin factory-intelligence
