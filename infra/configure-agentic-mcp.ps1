[CmdletBinding()]
param(
    [string]$AgenticDomain = $env:FACTORY_AGENTIC_DOMAIN,
    [string]$McpDomain = $env:FACTORY_MCP_DOMAIN,
    [string]$ClientId = $env:FACTORY_AGENTIC_CLIENT_ID,
    [string]$KnowledgeSourceName = "factory-maintenance-manuals",
    [string]$Token,
    [switch]$SkipCertificateCheck
)

$ErrorActionPreference = "Stop"

foreach ($entry in @{
    AgenticDomain = $AgenticDomain
    McpDomain = $McpDomain
    ClientId = $ClientId
}.GetEnumerator()) {
    if ([string]::IsNullOrWhiteSpace($entry.Value)) {
        throw "$($entry.Key) is required. Pass it as a parameter or environment variable."
    }
}

if (-not $Token) {
    $Token = az account get-access-token `
        --resource "api://$ClientId" `
        --query accessToken `
        --output tsv
    if (-not $Token) {
        throw "Failed to acquire an Agentic Retrieval access token."
    }
}

$baseUrl = "https://$AgenticDomain"
$headers = @{ Authorization = "Bearer $Token" }

function Invoke-AgenticJson {
    param(
        [Parameter(Mandatory)]
        [ValidateSet("GET", "POST", "PATCH")]
        [string]$Method,

        [Parameter(Mandatory)]
        [string]$Path,

        [hashtable]$Body
    )

    $parameters = @{
        Method = $Method
        Uri = "$baseUrl$Path"
        Headers = $headers
        ContentType = "application/json"
        TimeoutSec = 120
    }
    if ($SkipCertificateCheck) {
        $parameters.SkipCertificateCheck = $true
    }
    if ($Body) {
        $parameters.Body = $Body | ConvertTo-Json -Depth 20
    }
    Invoke-RestMethod @parameters
}

$existing = Invoke-AgenticJson `
    -Method GET `
    -Path "/edgeai/knowledgesources?name=$KnowledgeSourceName&limit=10"
$knowledgeSource = @($existing.items) | Select-Object -First 1

if (-not $knowledgeSource) {
    $knowledgeSource = Invoke-AgenticJson `
        -Method POST `
        -Path "/edgeai/knowledgesources" `
        -Body @{
            name = $KnowledgeSourceName
            kind = "remote_mcp"
            auth_type = "microsoft_entra_id"
            description = "Search synthetic factory maintenance manuals hosted on AKS."
            remote_mcp_parameters = @{
                server_url = "https://$McpDomain/mcp"
                server_label = "factory-maintenance-search"
            }
        }
}

if ($knowledgeSource.validation_status -ne "active") {
    throw "The MCP knowledge source is not active: $($knowledgeSource.validation_error)"
}

$knowledgeBases = Invoke-AgenticJson `
    -Method GET `
    -Path "/knowledge-bases?limit=10"
$knowledgeBase = @($knowledgeBases.data)[0]
if (-not $knowledgeBase) {
    throw "The default knowledge base was not returned."
}
$knowledgeBaseId = $knowledgeBase.id

$sourceIds = @($knowledgeBase.knowledge_source_ids)
if ($knowledgeSource.id -notin $sourceIds) {
    $sourceIds += $knowledgeSource.id
    Invoke-AgenticJson `
        -Method PATCH `
        -Path "/knowledge-bases/$knowledgeBaseId" `
        -Body @{ knowledge_source_ids = $sourceIds } | Out-Null
}

[pscustomobject]@{
    AgenticBaseUrl = $baseUrl
    McpEndpoint = "https://$McpDomain/mcp"
    KnowledgeSourceId = $knowledgeSource.id
    KnowledgeSourceStatus = $knowledgeSource.validation_status
    KnowledgeBaseId = $knowledgeBaseId
} | ConvertTo-Json -Depth 10
