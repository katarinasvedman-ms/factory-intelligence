[CmdletBinding()]
param(
    [string]$EdgeRagDisplayName = "Factory Intelligence EdgeRAG",
    [string]$FoundryDisplayName = "Factory Intelligence Foundry Local",
    [string]$DomainName = "arcrag.factory-intelligence.test",
    [string]$CollectionName = "factory-maintenance-demo"
)

$ErrorActionPreference = "Stop"

az account show --output none
$tenantId = az account show --query tenantId --output tsv
$currentUserId = az ad signed-in-user show --query id --output tsv

function Assert-AzSuccess {
    param([string]$Operation)
    if ($LASTEXITCODE -ne 0) {
        throw "$Operation failed with exit code $LASTEXITCODE."
    }
}

function Invoke-GraphJson {
    param(
        [string]$Method,
        [string]$Url,
        [hashtable]$Body
    )

    $temporaryFile = Join-Path ([System.IO.Path]::GetTempPath()) "graph-$([guid]::NewGuid()).json"
    try {
        $Body | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $temporaryFile -Encoding utf8
        $result = az rest `
            --method $Method `
            --url $Url `
            --headers "Content-Type=application/json" `
            --body "@$temporaryFile" `
            --output json
        Assert-AzSuccess "Microsoft Graph $Method $Url"
        return $result
    }
    finally {
        Remove-Item -LiteralPath $temporaryFile -Force -ErrorAction SilentlyContinue
    }
}

function Get-OrCreateApplication {
    param(
        [string]$DisplayName,
        [string]$SignInAudience
    )

    $app = az ad app list `
        --display-name $DisplayName `
        --query "[0].{appId:appId,id:id}" `
        --output json | ConvertFrom-Json
    Assert-AzSuccess "Query app registration $DisplayName"

    if (-not $app) {
        $app = az ad app create `
            --display-name $DisplayName `
            --sign-in-audience $SignInAudience `
            --query "{appId:appId,id:id}" `
            --output json | ConvertFrom-Json
        Assert-AzSuccess "Create app registration $DisplayName"
    }

    $servicePrincipal = az ad sp list `
        --filter "appId eq '$($app.appId)'" `
        --query "[0].{id:id}" `
        --output json | ConvertFrom-Json
    Assert-AzSuccess "Query service principal for $DisplayName"
    if (-not $servicePrincipal) {
        $servicePrincipal = az ad sp create `
            --id $app.appId `
            --query "{id:id}" `
            --output json | ConvertFrom-Json
        Assert-AzSuccess "Create service principal for $DisplayName"
    }

    [pscustomobject]@{
        AppId = $app.appId
        ObjectId = $app.id
        ServicePrincipalId = $servicePrincipal.id
    }
}

function New-AppRole {
    param(
        [string]$DisplayName,
        [string]$Value,
        [string[]]$AllowedMemberTypes,
        [object[]]$ExistingRoles = @()
    )

    $existing = $ExistingRoles | Where-Object { $_.value -eq $Value } | Select-Object -First 1
    @{
        allowedMemberTypes = $AllowedMemberTypes
        description = $DisplayName
        displayName = $DisplayName
        id = if ($existing) { $existing.id } else { [guid]::NewGuid().Guid }
        isEnabled = $true
        value = $Value
    }
}

function Set-ApplicationManifest {
    param(
        [string]$ObjectId,
        [hashtable]$Body
    )

    Invoke-GraphJson `
        -Method PATCH `
        -Url "https://graph.microsoft.com/v1.0/applications/$ObjectId" `
        -Body $Body | Out-Null
}

function Add-UserRoleAssignment {
    param(
        [string]$UserId,
        [string]$ServicePrincipalId,
        [string]$AppRoleId
    )

    $assignments = az rest `
        --method GET `
        --url "https://graph.microsoft.com/v1.0/users/$UserId/appRoleAssignments" `
        --output json | ConvertFrom-Json
    Assert-AzSuccess "Query app-role assignments for user $UserId"
    $existing = $assignments.value | Where-Object {
        $_.resourceId -eq $ServicePrincipalId -and $_.appRoleId -eq $AppRoleId
    } | Select-Object -First 1
    if (-not $existing) {
        Invoke-GraphJson `
            -Method POST `
            -Url "https://graph.microsoft.com/v1.0/users/$UserId/appRoleAssignments" `
            -Body @{
                principalId = $UserId
                resourceId = $ServicePrincipalId
                appRoleId = $AppRoleId
            } | Out-Null
    }
}

$edgeRag = Get-OrCreateApplication `
    -DisplayName $EdgeRagDisplayName `
    -SignInAudience AzureADMultipleOrgs

$edgeRagManifest = az rest `
    --method GET `
    --url "https://graph.microsoft.com/v1.0/applications/$($edgeRag.ObjectId)?`$select=appRoles" `
    --output json | ConvertFrom-Json
Assert-AzSuccess "Read Edge RAG application roles"
$edgeRagExistingRoles = @($edgeRagManifest.appRoles)

$edgeRagRoles = @(
    (New-AppRole -DisplayName "EdgeRAGDeveloper" -Value "EdgeRAGDeveloper" -AllowedMemberTypes @("User") -ExistingRoles $edgeRagExistingRoles),
    (New-AppRole -DisplayName "EdgeRAGEndUser" -Value "EdgeRAGEndUser" -AllowedMemberTypes @("User") -ExistingRoles $edgeRagExistingRoles),
    (New-AppRole -DisplayName "Default collection" -Value "edgeragapp" -AllowedMemberTypes @("User") -ExistingRoles $edgeRagExistingRoles),
    (New-AppRole -DisplayName "Factory maintenance collection" -Value $CollectionName -AllowedMemberTypes @("User") -ExistingRoles $edgeRagExistingRoles)
)

Set-ApplicationManifest `
    -ObjectId $edgeRag.ObjectId `
    -Body @{
        signInAudience = "AzureADMultipleOrgs"
        spa = @{
            redirectUris = @("https://$DomainName/")
        }
        publicClient = @{
            redirectUris = @("https://login.microsoftonline.com/common/oauth2/nativeclient")
        }
        appRoles = $edgeRagRoles
    }

foreach ($role in $edgeRagRoles) {
    Add-UserRoleAssignment `
        -UserId $currentUserId `
        -ServicePrincipalId $edgeRag.ServicePrincipalId `
        -AppRoleId $role.id
}

$foundry = Get-OrCreateApplication `
    -DisplayName $FoundryDisplayName `
    -SignInAudience AzureADMyOrg

$foundryManifest = az rest `
    --method GET `
    --url "https://graph.microsoft.com/v1.0/applications/$($foundry.ObjectId)?`$select=appRoles,api" `
    --output json | ConvertFrom-Json
Assert-AzSuccess "Read Foundry Local application manifest"
$existingFoundryScope = @($foundryManifest.api.oauth2PermissionScopes) |
    Where-Object { $_.value -eq "foundry_access" } |
    Select-Object -First 1
$foundryScopeId = if ($existingFoundryScope) {
    $existingFoundryScope.id
}
else {
    [guid]::NewGuid().Guid
}
$foundryAccessRole = New-AppRole `
    -DisplayName "Foundry inference access" `
    -Value "FoundryInferenceAccess" `
    -AllowedMemberTypes @("Application") `
    -ExistingRoles @($foundryManifest.appRoles)

Set-ApplicationManifest `
    -ObjectId $foundry.ObjectId `
    -Body @{
        signInAudience = "AzureADMyOrg"
        identifierUris = @("api://$($foundry.AppId)")
        api = @{
            requestedAccessTokenVersion = 2
            oauth2PermissionScopes = @(
                @{
                    adminConsentDescription = "Allows access to Foundry Local inference endpoints."
                    adminConsentDisplayName = "Access Foundry Local inference endpoints"
                    id = $foundryScopeId
                    isEnabled = $true
                    type = "Admin"
                    userConsentDescription = "Access Foundry Local inference endpoints."
                    userConsentDisplayName = "Access Foundry Local inference endpoints"
                    value = "foundry_access"
                }
            )
        }
        appRoles = @($foundryAccessRole)
    }

Set-ApplicationManifest `
    -ObjectId $foundry.ObjectId `
    -Body @{
        api = @{
            requestedAccessTokenVersion = 2
            oauth2PermissionScopes = @(
                @{
                    adminConsentDescription = "Allows access to Foundry Local inference endpoints."
                    adminConsentDisplayName = "Access Foundry Local inference endpoints"
                    id = $foundryScopeId
                    isEnabled = $true
                    type = "Admin"
                    userConsentDescription = "Access Foundry Local inference endpoints."
                    userConsentDisplayName = "Access Foundry Local inference endpoints"
                    value = "foundry_access"
                }
            )
            preAuthorizedApplications = @(
                @{
                    appId = "04b07795-8ddb-461a-bbee-02f9e1bf7b46"
                    delegatedPermissionIds = @($foundryScopeId)
                }
            )
        }
    }

[pscustomobject]@{
    TenantId = $tenantId
    DomainName = $DomainName
    EdgeRagClientId = $edgeRag.AppId
    EdgeRagObjectId = $edgeRag.ObjectId
    EdgeRagServicePrincipalId = $edgeRag.ServicePrincipalId
    FoundryClientId = $foundry.AppId
    FoundryObjectId = $foundry.ObjectId
    FoundryServicePrincipalId = $foundry.ServicePrincipalId
    FoundryInferenceAccessRoleId = $foundryAccessRole.id
    CollectionName = $CollectionName
} | ConvertTo-Json -Depth 5
