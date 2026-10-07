[CmdletBinding()]
param(
    [string]$AgenticDomain,
    [string]$ClientId,
    [string]$FoundryBaseUrl,
    [string]$FoundryExpertModel,
    [string]$ResourceGroup,
    [string]$ClusterName,
    [string]$LocalConfigPath = ".env.local",
    [string]$ConfigPath = "config\demo.aks-agentic.yaml",
    [ValidateRange(60, 1800)]
    [int]$AgenticStartupTimeoutSeconds = 900
)

$ErrorActionPreference = "Stop"

function Import-LocalEnvironment {
    param([string]$Path)

    if ([string]::IsNullOrWhiteSpace($Path)) {
        return
    }

    $resolvedPath = if ([System.IO.Path]::IsPathRooted($Path)) {
        $Path
    }
    else {
        Join-Path $PSScriptRoot $Path
    }

    if (-not (Test-Path -LiteralPath $resolvedPath)) {
        return
    }

    foreach ($line in Get-Content -LiteralPath $resolvedPath) {
        $trimmed = $line.Trim()
        if (-not $trimmed -or $trimmed.StartsWith("#")) {
            continue
        }

        $separator = $trimmed.IndexOf("=")
        if ($separator -le 0) {
            throw "Invalid local configuration line in ${resolvedPath}: $line"
        }

        $name = $trimmed.Substring(0, $separator).Trim()
        $value = $trimmed.Substring($separator + 1).Trim()
        if ($name -notmatch "^[A-Za-z_][A-Za-z0-9_]*$") {
            throw "Invalid environment variable name '$name' in $resolvedPath."
        }

        if ($value.Length -ge 2) {
            $first = $value[0]
            $last = $value[$value.Length - 1]
            if (($first -eq '"' -and $last -eq '"') -or ($first -eq "'" -and $last -eq "'")) {
                $value = $value.Substring(1, $value.Length - 2)
            }
        }

        if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name, "Process"))) {
            [Environment]::SetEnvironmentVariable($name, $value, "Process")
        }
    }

    Write-Host "Loaded local configuration from $resolvedPath"
}

Import-LocalEnvironment -Path $LocalConfigPath

if ([string]::IsNullOrWhiteSpace($AgenticDomain)) {
    $AgenticDomain = $env:FACTORY_AGENTIC_DOMAIN
}
if ([string]::IsNullOrWhiteSpace($ClientId)) {
    $ClientId = $env:FACTORY_AGENTIC_CLIENT_ID
}
if ([string]::IsNullOrWhiteSpace($FoundryBaseUrl)) {
    $FoundryBaseUrl = $env:FOUNDRY_BASE_URL
}
if ([string]::IsNullOrWhiteSpace($FoundryExpertModel)) {
    $FoundryExpertModel = $env:FOUNDRY_EXPERT_MODEL
}
if ([string]::IsNullOrWhiteSpace($ResourceGroup)) {
    $ResourceGroup = $env:FACTORY_RESOURCE_GROUP
}
if ([string]::IsNullOrWhiteSpace($ClusterName)) {
    $ClusterName = $env:FACTORY_CLUSTER_NAME
}

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

$startedCluster = $false
if (-not [string]::IsNullOrWhiteSpace($ResourceGroup) -and -not [string]::IsNullOrWhiteSpace($ClusterName)) {
    $clusterState = az aks show `
        --resource-group $ResourceGroup `
        --name $ClusterName `
        --query powerState.code `
        --output tsv `
        --only-show-errors
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($clusterState)) {
        throw "Unable to read AKS cluster state for $ResourceGroup/$ClusterName."
    }

    if ($clusterState.Trim() -eq "Stopped") {
        Write-Host "Starting AKS cluster $ResourceGroup/$ClusterName..."
        az aks start `
            --resource-group $ResourceGroup `
            --name $ClusterName `
            --only-show-errors
        if ($LASTEXITCODE -ne 0) {
            throw "Unable to start AKS cluster $ResourceGroup/$ClusterName."
        }
        $startedCluster = $true
    }
    elseif ($clusterState.Trim() -ne "Running") {
        throw "AKS cluster $ResourceGroup/$ClusterName is in unsupported power state '$clusterState'."
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
$startupDeadline = [DateTime]::UtcNow.AddSeconds($AgenticStartupTimeoutSeconds)
do {
    try {
        $knowledgeBases = Invoke-RestMethod `
            -Uri "$baseUrl/knowledge-bases?limit=1" `
            -Headers @{ Authorization = "Bearer $edgeToken" } `
            -TimeoutSec 60
        break
    }
    catch {
        if (-not $startedCluster -or [DateTime]::UtcNow -ge $startupDeadline) {
            throw
        }

        Write-Host "Waiting for Agentic Retrieval to become available..."
        Start-Sleep -Seconds 15
    }
} while ($true)

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
