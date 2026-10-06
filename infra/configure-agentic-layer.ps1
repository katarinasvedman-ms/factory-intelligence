[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$DomainName,

    [Parameter(Mandatory)]
    [string]$ClientId,

    [Parameter(Mandatory)]
    [string]$NfsServerPath,

    [string]$CollectionName = "factory-maintenance-demo",
    [string]$KnowledgeSourceName = "factory-maintenance-manuals",
    [string]$IngestionJobId = "factory-maintenance-ingest",
    [string]$KnowledgeSourceId,
    [string]$Token
)

$ErrorActionPreference = "Stop"

if (-not $Token) {
    $Token = az account get-access-token `
        --resource "api://$ClientId" `
        --query accessToken `
        --output tsv
    if ($LASTEXITCODE -ne 0 -or -not $Token) {
        throw "Failed to acquire an Agentic Retrieval access token."
    }
}

$baseUrl = "https://$DomainName"
$headers = @{ Authorization = "Bearer $Token" }
$apiVersion = "2024-10-01-preview"

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
    }
    if ($Body) {
        $parameters.Body = $Body | ConvertTo-Json -Depth 20
    }
    Invoke-RestMethod @parameters
}

try {
    Invoke-AgenticJson `
        -Method GET `
        -Path "/edgeai/collections/$CollectionName`?api-version=$apiVersion" | Out-Null
    Write-Host "Collection already exists: $CollectionName"
}
catch {
    if ($_.Exception.Response.StatusCode.value__ -ne 404) {
        throw
    }
    Invoke-AgenticJson `
        -Method POST `
        -Path "/edgeai/collections?api-version=$apiVersion" `
        -Body @{
            name = $CollectionName
            description = "Synthetic factory maintenance manuals and incident guidance."
        } | Out-Null
    Write-Host "Created collection: $CollectionName"
}

$ingestion = Invoke-AgenticJson `
    -Method POST `
    -Path "/edgeai/ingestion/jobs/$IngestionJobId`?api-version=$apiVersion" `
    -Body @{
        datasource = @{
            kind = "nfsv3"
            connection = @{
                nfsServerPath = $NfsServerPath
                nfsUid = 0
                nfsGid = 0
            }
            chunking = @{
                chunkSize = "2000"
                chunkOverlap = "200"
            }
        }
        collectionName = $CollectionName
        dataRefreshIntervalInHours = -1
        parsingMode = "advanced"
    }
Write-Host "Submitted ingestion job: $IngestionJobId"

if (-not $KnowledgeSourceId) {
    $knowledgeSource = Invoke-AgenticJson `
        -Method POST `
        -Path "/edgeai/knowledgesources" `
        -Body @{
            name = $KnowledgeSourceName
            kind = "indexed_sources_mcp"
            auth_type = "microsoft_entra_id"
            description = "Search the synthetic factory maintenance collection."
            indexed_sources_parameters = @{
                server_url = "$baseUrl/edgeai/mcp"
                indexed_source_ref = $CollectionName
                server_label = "factory-maintenance-search"
            }
        }
    $KnowledgeSourceId = $knowledgeSource.id
    Write-Host "Created knowledge source: $KnowledgeSourceId"
}

$knowledgeBases = Invoke-AgenticJson `
    -Method GET `
    -Path "/knowledge-bases?limit=1"
$knowledgeBase = @($knowledgeBases.data)[0]
if (-not $knowledgeBase) {
    throw "The default knowledge base was not returned."
}

$sourceIds = @($knowledgeBase.knowledge_source_ids)
if ($KnowledgeSourceId -notin $sourceIds) {
    $sourceIds += $KnowledgeSourceId
    $knowledgeBase = Invoke-AgenticJson `
        -Method PATCH `
        -Path "/knowledge-bases/$($knowledgeBase.id)" `
        -Body @{ knowledge_source_ids = $sourceIds }
}

[pscustomobject]@{
    CollectionName = $CollectionName
    KnowledgeSourceId = $KnowledgeSourceId
    AgentId = $knowledgeBase.id
    IngestionJobId = $IngestionJobId
    IngestionStatus = $ingestion.status
    BaseUrl = $baseUrl
} | ConvertTo-Json -Depth 10
