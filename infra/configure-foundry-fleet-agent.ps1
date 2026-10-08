[CmdletBinding()]
param(
    [string]$SubscriptionId = $(az account show --query id --output tsv),
    [string]$ResourceGroup = "factory-intelligence",
    [string]$FoundryAccount = "factory-intelligence-resource",
    [string]$FoundryProject = "factory-intelligence",
    [string]$ProjectEndpoint,
    [string]$Model,
    [string]$WorkspaceId,
    [string]$DataAgentId,
    [string]$KqlDatabaseId,
    [string]$ConnectionName = "factory-fabric-iq",
    [string]$AgentName = "factory-fleet-manager",
    [switch]$SkipFoundryAgent,
    [switch]$SmokeTest
)

$ErrorActionPreference = "Stop"

$localEnvironment = Join-Path (Split-Path $PSScriptRoot -Parent) ".env.local"
if (Test-Path -LiteralPath $localEnvironment) {
    foreach ($line in Get-Content -LiteralPath $localEnvironment) {
        $trimmed = $line.Trim()
        if (-not $trimmed -or $trimmed.StartsWith("#") -or -not $trimmed.Contains("=")) {
            continue
        }
        $name, $value = $trimmed.Split("=", 2)
        if (-not [Environment]::GetEnvironmentVariable($name, "Process")) {
            [Environment]::SetEnvironmentVariable($name, $value, "Process")
        }
    }
}

if (-not $ProjectEndpoint) {
    $ProjectEndpoint = $env:FOUNDRY_PROJECT_ENDPOINT
}
if (-not $ProjectEndpoint) {
    $ProjectEndpoint =
        "https://$FoundryAccount.services.ai.azure.com/api/projects/$FoundryProject"
}
if (-not $Model) {
    $Model = if ($env:FOUNDRY_AGENT_MODEL) {
        $env:FOUNDRY_AGENT_MODEL
    }
    else {
        "factory-expert-gpt54"
    }
}
if (-not $WorkspaceId) {
    $WorkspaceId = $env:FABRIC_WORKSPACE_ID
}
if (-not $DataAgentId) {
    $DataAgentId = $env:FABRIC_DATA_AGENT_ID
}
if (-not $KqlDatabaseId) {
    $KqlDatabaseId = $env:FABRIC_KQL_DATABASE_ID
}
foreach ($required in @{
        SubscriptionId = $SubscriptionId
        WorkspaceId = $WorkspaceId
        DataAgentId = $DataAgentId
        KqlDatabaseId = $KqlDatabaseId
    }.GetEnumerator()) {
    if (-not $required.Value) {
        throw "$($required.Key) must be supplied as a parameter or environment variable."
    }
}

function ConvertTo-Base64 {
    param([string]$Value)
    [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($Value))
}

az account show --output none
if ($LASTEXITCODE -ne 0) {
    throw "Azure CLI authentication is required."
}

$fabricToken = az account get-access-token `
    --resource https://api.fabric.microsoft.com `
    --query accessToken `
    --output tsv
$fabricHeaders = @{
    Authorization = "Bearer $fabricToken"
    "Content-Type" = "application/json"
}
$dataAgentBase =
    "https://api.fabric.microsoft.com/v1/workspaces/$WorkspaceId/dataAgents/$DataAgentId"

foreach ($column in @("summary", "details")) {
    $elementId = ConvertTo-Base64 "Tables/factory_incident_events/$column"
    $encodedId = [uri]::EscapeDataString($elementId)
    Invoke-RestMethod `
        -Method Patch `
        -Uri "$dataAgentBase/staging/datasources/$KqlDatabaseId/elements?id=$encodedId" `
        -Headers $fabricHeaders `
        -Body '{"isSelected":false}' | Out-Null
}

$agentSettings = @{
    aiInstructions = "You are a governed analytics query service. Return only facts from query results in compact Markdown tables or concise bullet lists. Do not provide recommendations, risk narratives, operational advice, causal speculation, or machine-control guidance. Distinguish lifecycle event rows from distinct incidents. State the represented data period and scope. Never expose operator identities or raw alarm text. If a query fails, return only the concise technical error and the failed query shape."
} | ConvertTo-Json
Invoke-RestMethod `
    -Method Patch `
    -Uri "$dataAgentBase/staging/settings" `
    -Headers $fabricHeaders `
    -Body $agentSettings | Out-Null

$datasourceInstructions = @{
    instructions = "Use factory_incident_events as an append-only lifecycle event table. Query only structured columns; summary and details are intentionally excluded. Return compact factual tables without recommendations or risk language. Begin analytical queries by deduplicating event_id with summarize arg_max(occurred_at, *) by event_id because a demo seed can republish the same logical events with refreshed timestamps. Count distinct incident_id values when counting incidents; event rows are not incidents. For time-window briefs, anchor the window to max(occurred_at), use the validated decomposed few-shots, and keep dcount as a direct summarize aggregation; never nest dcount inside make_bag, pivot, evaluate, or another aggregate. Determine current incident state using arg_max(occurred_at, *) by incident_id after event-id deduplication. State the represented time range and factory scope. Treat event_type values as neutral lifecycle labels and do not infer causes or physical outcomes."
} | ConvertTo-Json
Invoke-RestMethod `
    -Method Patch `
    -Uri "$dataAgentBase/staging/datasources/$KqlDatabaseId" `
    -Headers $fabricHeaders `
    -Body $datasourceInstructions | Out-Null

$fewshots = Invoke-RestMethod `
    -Uri "$dataAgentBase/staging/datasources/$KqlDatabaseId/fewshots" `
    -Headers $fabricHeaders

$examples = @(
    @{
        question = "For the latest 24-hour period represented in the data, return the represented time range, lifecycle event row count, distinct incident count, and the same counts for the preceding 24 hours."
        query = 'let events = factory_incident_events | summarize arg_max(occurred_at, *) by event_id; let max_time = toscalar(events | summarize max(occurred_at)); events | where occurred_at > max_time - 48h and occurred_at <= max_time | extend period = iff(occurred_at > max_time - 24h, "current_24h", "previous_24h") | summarize event_rows=count(), distinct_incidents=dcount(incident_id), represented_start=min(occurred_at), represented_end=max(occurred_at) by period | order by period asc'
    },
    @{
        question = "For the latest 24-hour period represented in the data, count incidents by their latest incident status."
        query = 'let events = factory_incident_events | summarize arg_max(occurred_at, *) by event_id; let max_time = toscalar(events | summarize max(occurred_at)); events | where occurred_at > max_time - 24h and occurred_at <= max_time | summarize arg_max(occurred_at, *) by incident_id | summarize incidents=count() by incident_status | order by incidents desc'
    },
    @{
        question = "For the latest 24-hour period represented in the data, count lifecycle events and affected incidents by event type, including governed actions."
        query = 'let events = factory_incident_events | summarize arg_max(occurred_at, *) by event_id; let max_time = toscalar(events | summarize max(occurred_at)); events | where occurred_at > max_time - 24h and occurred_at <= max_time | summarize event_rows=count(), distinct_incidents=dcount(incident_id) by event_type | order by event_rows desc'
    },
    @{
        question = "Show governed actions and downstream monitoring events in the latest 24-hour period represented in the data."
        query = 'let events = factory_incident_events | summarize arg_max(occurred_at, *) by event_id; let max_time = toscalar(events | summarize max(occurred_at)); events | where occurred_at > max_time - 24h and occurred_at <= max_time | where event_type in ("guard_executed", "guard_rejected", "operator_rejected", "upstream_action_executed") | project occurred_at, factory_id, machine_id, incident_id, event_type, incident_status, severity, correlation_id | order by occurred_at desc'
    },
    @{
        question = "For the latest seven-day period represented in the data, return the represented time range, lifecycle event row count, distinct incident count, factory count, and the same counts for the preceding seven days."
        query = 'let events = factory_incident_events | summarize arg_max(occurred_at, *) by event_id; let max_time = toscalar(events | summarize max(occurred_at)); events | where occurred_at > max_time - 14d and occurred_at <= max_time | extend period = iff(occurred_at > max_time - 7d, "current_7d", "previous_7d") | summarize event_rows=count(), distinct_incidents=dcount(incident_id), factories=dcount(factory_id), represented_start=min(occurred_at), represented_end=max(occurred_at) by period | order by period asc'
    },
    @{
        question = "For the latest seven-day period represented in the data, count incidents by factory and their latest incident status."
        query = 'let events = factory_incident_events | summarize arg_max(occurred_at, *) by event_id; let max_time = toscalar(events | summarize max(occurred_at)); events | where occurred_at > max_time - 7d and occurred_at <= max_time | summarize arg_max(occurred_at, *) by incident_id | summarize incidents=count() by factory_id, incident_status | order by factory_id asc, incidents desc'
    },
    @{
        question = "For the latest seven-day period represented in the data, show detected event types that affected more than one factory."
        query = 'let events = factory_incident_events | summarize arg_max(occurred_at, *) by event_id; let max_time = toscalar(events | summarize max(occurred_at)); events | where occurred_at > max_time - 7d and occurred_at <= max_time | where event_type endswith "_detected" | summarize event_rows=count(), distinct_incidents=dcount(incident_id), factories=dcount(factory_id), factory_list=make_set(factory_id, 10) by event_type | where factories > 1 | order by factories desc, distinct_incidents desc'
    },
    @{
        question = "For the latest seven-day period represented in the data, show governed action outcomes by factory."
        query = 'let events = factory_incident_events | summarize arg_max(occurred_at, *) by event_id; let max_time = toscalar(events | summarize max(occurred_at)); events | where occurred_at > max_time - 7d and occurred_at <= max_time | where event_type in ("guard_executed", "guard_rejected", "operator_rejected", "upstream_action_executed") | summarize event_rows=count(), distinct_incidents=dcount(incident_id) by factory_id, event_type | order by factory_id asc, event_rows desc'
    },
    @{
        question = "For the latest seven-day period represented in the data, show correlated incident activity by factory and upstream correlation ID."
        query = 'let events = factory_incident_events | summarize arg_max(occurred_at, *) by event_id; let max_time = toscalar(events | summarize max(occurred_at)); events | where occurred_at > max_time - 7d and occurred_at <= max_time | where isnotempty(correlation_id) | summarize event_rows=count(), distinct_incidents=dcount(incident_id), latest_event=max(occurred_at) by factory_id, correlation_id | order by latest_event desc'
    }
)
foreach ($example in $examples) {
    $existingFewshot = @($fewshots.value) |
        Where-Object question -eq $example.question |
        Select-Object -First 1
    $body = $example | ConvertTo-Json
    if ($existingFewshot) {
        Invoke-RestMethod `
            -Method Patch `
            -Uri "$dataAgentBase/staging/datasources/$KqlDatabaseId/fewshots/$($existingFewshot.id)" `
            -Headers $fabricHeaders `
            -Body $body | Out-Null
    }
    else {
        Invoke-RestMethod `
            -Method Post `
            -Uri "$dataAgentBase/staging/datasources/$KqlDatabaseId/fewshots" `
            -Headers $fabricHeaders `
            -Body $body | Out-Null
    }
}

$publishBody = @{
    publishedDescription =
        "Governed Floor structured fleet query service for Foundry brief synthesis."
} | ConvertTo-Json
Invoke-RestMethod `
    -Method Post `
    -Uri "$dataAgentBase/staging/publish" `
    -Headers $fabricHeaders `
    -Body $publishBody `
    -TimeoutSec 180 | Out-Null

if ($SkipFoundryAgent) {
    [pscustomobject]@{
        WorkspaceId = $WorkspaceId
        DataAgentId = $DataAgentId
        KqlDatabaseId = $KqlDatabaseId
        DataAgentPublished = $true
        FoundryAgentUpdated = $false
    } | ConvertTo-Json
    return
}

$fabricIqTarget =
    "https://api.fabric.microsoft.com/v1/mcp/workspaces/$WorkspaceId/dataagents/$DataAgentId/agent"
$armToken = az account get-access-token `
    --resource https://management.azure.com `
    --query accessToken `
    --output tsv
$armHeaders = @{
    Authorization = "Bearer $armToken"
    "Content-Type" = "application/json"
}
$connectionUri =
    "https://management.azure.com/subscriptions/$SubscriptionId/resourceGroups/$ResourceGroup/providers/Microsoft.CognitiveServices/accounts/$FoundryAccount/projects/$FoundryProject/connections/${ConnectionName}?api-version=2025-10-01-preview"
$connectionBody = @{
    properties = @{
        category = "RemoteTool"
        authType = "UserEntraToken"
        target = $fabricIqTarget
        audience = "https://analysis.windows.net/powerbi/api"
        isSharedToAll = $true
    }
} | ConvertTo-Json -Depth 8
Invoke-RestMethod `
    -Method Put `
    -Uri $connectionUri `
    -Headers $armHeaders `
    -Body $connectionBody | Out-Null

$pythonArguments = @(
    ".\infra\create-foundry-fleet-agent.py",
    "--project-endpoint", $ProjectEndpoint,
    "--model", $Model,
    "--connection-name", $ConnectionName,
    "--agent-name", $AgentName
)
if ($SmokeTest) {
    $pythonArguments += "--smoke-test"
}
python @pythonArguments
if ($LASTEXITCODE -ne 0) {
    throw "Foundry fleet agent creation failed with exit code $LASTEXITCODE."
}
