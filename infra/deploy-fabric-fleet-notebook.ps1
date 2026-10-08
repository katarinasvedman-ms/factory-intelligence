[CmdletBinding()]
param(
    [string]$WorkspaceId = $env:FABRIC_WORKSPACE_ID,
    [string]$KqlDatabaseId = $env:FABRIC_KQL_DATABASE_ID,
    [string]$NotebookName = "Seed Governed Floor Fleet Week",
    [string]$LocalConfigPath = ".env.local",
    [switch]$Run
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

function Import-LocalEnvironment {
    param([string]$Path)

    $resolved = if ([IO.Path]::IsPathRooted($Path)) {
        $Path
    }
    else {
        Join-Path (Split-Path $PSScriptRoot -Parent) $Path
    }
    if (-not (Test-Path -LiteralPath $resolved)) {
        return
    }

    foreach ($line in Get-Content -LiteralPath $resolved) {
        $trimmed = $line.Trim()
        if (-not $trimmed -or $trimmed.StartsWith("#") -or -not $trimmed.Contains("=")) {
            continue
        }
        $name, $value = $trimmed.Split("=", 2)
        if (-not [Environment]::GetEnvironmentVariable($name, "Process")) {
            [Environment]::SetEnvironmentVariable($name, $value.Trim(), "Process")
        }
    }
}

function Wait-FabricOperation {
    param(
        [string]$Location,
        [hashtable]$Headers,
        [int]$TimeoutMinutes = 15
    )

    if (-not $Location) {
        return
    }
    $deadline = [DateTime]::UtcNow.AddMinutes($TimeoutMinutes)
    do {
        Start-Sleep -Seconds 5
        $operation = Invoke-RestMethod -Uri $Location -Headers $Headers
        if ($operation.status -in @("Succeeded", "Completed")) {
            return $operation
        }
        if ($operation.status -in @("Failed", "Cancelled")) {
            throw "Fabric operation ended with status $($operation.status): $($operation.error | ConvertTo-Json -Compress)"
        }
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Fabric operation did not complete within $TimeoutMinutes minutes."
}

function Invoke-FabricMutation {
    param(
        [ValidateSet("Post")]
        [string]$Method,
        [string]$Uri,
        [hashtable]$Headers,
        [string]$Body
    )

    $response = Invoke-WebRequest `
        -Method $Method `
        -Uri $Uri `
        -Headers $Headers `
        -ContentType "application/json" `
        -Body $Body
    if ($response.StatusCode -eq 202) {
        Wait-FabricOperation -Location $response.Headers.Location -Headers $Headers | Out-Null
    }
    elseif ($response.StatusCode -notin @(200, 201)) {
        throw "Fabric request returned HTTP $($response.StatusCode)."
    }
}

Import-LocalEnvironment -Path $LocalConfigPath
if (-not $WorkspaceId) { $WorkspaceId = $env:FABRIC_WORKSPACE_ID }
if (-not $KqlDatabaseId) { $KqlDatabaseId = $env:FABRIC_KQL_DATABASE_ID }
foreach ($entry in @{
    WorkspaceId = $WorkspaceId
    KqlDatabaseId = $KqlDatabaseId
}.GetEnumerator()) {
    if (-not $entry.Value) {
        throw "$($entry.Key) must be supplied or configured in $LocalConfigPath."
    }
}

az account show --output none
if ($LASTEXITCODE -ne 0) {
    throw "Azure CLI authentication is required."
}
$token = az account get-access-token `
    --resource https://api.fabric.microsoft.com `
    --query accessToken `
    --output tsv
$headers = @{ Authorization = "Bearer $token" }
$baseUri = "https://api.fabric.microsoft.com/v1/workspaces/$WorkspaceId"

$database = Invoke-RestMethod `
    -Uri "$baseUri/kqlDatabases/$KqlDatabaseId" `
    -Headers $headers
$sourcePath = Join-Path (Split-Path $PSScriptRoot -Parent) "fabric\notebooks\seed-factory-fleet.py"
$source = Get-Content -LiteralPath $sourcePath -Raw
$source = $source.
    Replace("__KUSTO_URI__", $database.properties.queryServiceUri).
    Replace("__KUSTO_DATABASE__", $database.displayName)
$payload = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($source))
$definition = @{
    format = "fabricGitSource"
    parts = @(
        @{
            path = "notebook-content.py"
            payload = $payload
            payloadType = "InlineBase64"
        }
    )
}

$notebooks = Invoke-RestMethod -Uri "$baseUri/notebooks" -Headers $headers
$notebook = @($notebooks.value) |
    Where-Object displayName -eq $NotebookName |
    Select-Object -First 1
if ($notebook) {
    $body = @{ definition = $definition } | ConvertTo-Json -Depth 8
    Invoke-FabricMutation `
        -Method Post `
        -Uri "$baseUri/notebooks/$($notebook.id)/updateDefinition" `
        -Headers $headers `
        -Body $body
    $notebookId = $notebook.id
}
else {
    $body = @{
        displayName = $NotebookName
        description = "Refreshes a small relative seven-day Governed Floor fleet history for demonstrations."
        definition = $definition
    } | ConvertTo-Json -Depth 8
    Invoke-FabricMutation `
        -Method Post `
        -Uri "$baseUri/notebooks" `
        -Headers $headers `
        -Body $body
    $notebooks = Invoke-RestMethod -Uri "$baseUri/notebooks" -Headers $headers
    $notebookId = @($notebooks.value) |
        Where-Object displayName -eq $NotebookName |
        Select-Object -ExpandProperty id -First 1
}
if (-not $notebookId) {
    throw "Unable to resolve the deployed notebook ID."
}

$runResult = $null
if ($Run) {
    $runResponse = Invoke-WebRequest `
        -Method Post `
        -Uri "$baseUri/notebooks/$notebookId/jobs/execute/instances?beta=false" `
        -Headers $headers `
        -ContentType "application/json" `
        -Body '{"executionData":{"compute":"Spark"}}'
    if ($runResponse.StatusCode -ne 202) {
        throw "Notebook execution returned HTTP $($runResponse.StatusCode)."
    }
    $runResult = Wait-FabricOperation `
        -Location $runResponse.Headers.Location `
        -Headers $headers `
        -TimeoutMinutes 30
}

[pscustomobject]@{
    WorkspaceId = $WorkspaceId
    KqlDatabaseId = $KqlDatabaseId
    KqlDatabaseName = $database.displayName
    NotebookId = $notebookId
    NotebookName = $NotebookName
    Executed = [bool]$Run
    RunStatus = $runResult.status
    ExitValue = $runResult.exitValue
} | ConvertTo-Json
