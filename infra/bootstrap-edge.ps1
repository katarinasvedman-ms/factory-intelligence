[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$ResourceGroup,

    [Parameter(Mandatory)]
    [string]$ClusterName,

    [Parameter(Mandatory)]
    [string]$Location,

    [Parameter(Mandatory)]
    [string]$DomainName,

    [Parameter(Mandatory)]
    [string]$EdgeRagClientId,

    [Parameter(Mandatory)]
    [string]$FoundryClientId,

    [string]$TenantId,
    [string]$ModelName = "gpt-oss-20b",
    [string]$GpuNodePoolName = "gpu",
    [string]$AgenticRetrievalExtensionName = "edgeragdemo",
    [ValidateSet("combined", "agentic", "knowledge")]
    [string]$LayerSelection = "combined",
    [string]$NvidiaOperatorVersion = "v24.9.2",
    [switch]$PlatformOnly
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

if (-not $TenantId) {
    $TenantId = az account show --query tenantId --output tsv
}

$requiredCommands = @("az", "kubectl")
if (-not $PlatformOnly) {
    $requiredCommands += "helm"
}
foreach ($command in $requiredCommands) {
    if (-not (Get-Command $command -ErrorAction SilentlyContinue)) {
        throw "$command is required."
    }
}

az extension add --name connectedk8s --upgrade --yes
az extension add --name k8s-extension --upgrade --yes

foreach ($provider in @(
    "Microsoft.Kubernetes",
    "Microsoft.KubernetesConfiguration",
    "Microsoft.ExtendedLocation"
)) {
    az provider register --namespace $provider --wait --output none
}

az aks get-credentials `
    --resource-group $ResourceGroup `
    --name $ClusterName `
    --admin `
    --overwrite-existing

if (-not $PlatformOnly) {
    helm repo add nvidia https://helm.ngc.nvidia.com/nvidia
    helm repo update
    $gpuOperator = helm list --namespace gpu-operator --output json | ConvertFrom-Json
    if (-not $gpuOperator) {
        helm install nvidia-gpu-operator nvidia/gpu-operator `
            --namespace gpu-operator `
            --create-namespace `
            --version $NvidiaOperatorVersion `
            --wait
    }
}

$connectedCluster = az connectedk8s list `
    --resource-group $ResourceGroup `
    --query "[?name=='$ClusterName'] | [0]" `
    --output json | ConvertFrom-Json

if (-not $connectedCluster) {
    az connectedk8s connect `
        --resource-group $ResourceGroup `
        --location $Location `
        --name $ClusterName
}

$certificateExtension = az k8s-extension list `
    --resource-group $ResourceGroup `
    --cluster-type connectedClusters `
    --cluster-name $ClusterName `
    --query "[?name=='azure-cert-manager'] | [0]" `
    --output json | ConvertFrom-Json

if (-not $certificateExtension) {
    az k8s-extension create `
        --cluster-name $ClusterName `
        --name azure-cert-manager `
        --resource-group $ResourceGroup `
        --cluster-type connectedClusters `
        --extension-type Microsoft.CertManagement `
        --scope cluster `
        --release-train stable `
        --config config.enableGatewayAPI=true `
        --config cert-manager.crds.keep=true `
        --config trust-manager.defaultPackage.enabled=false `
        --config trust-manager.secretTargets.enabled=true `
        --config trust-manager.secretTargets.authorizedSecretsAll=true `
        --only-show-errors
}

if ($PlatformOnly) {
    Write-Host ""
    Write-Host "Arc connection and certificate platform bootstrap completed."
    Write-Warning "GPU-dependent Foundry Local and Agentic Retrieval components were skipped."
    return
}

az k8s-extension create `
    --resource-group $ResourceGroup `
    --cluster-name $ClusterName `
    --name inference-operator `
    --extension-type Microsoft.Foundry `
    --scope cluster `
    --release-namespace foundry-local-operator `
    --cluster-type connectedClusters `
    --auto-upgrade-minor-version true `
    --release-train stable `
    --config "entraAuth.tenantId=$TenantId" `
    --config "entraAuth.clientId=$FoundryClientId" `
    --only-show-errors

kubectl wait `
    --namespace foundry-local-operator `
    --for=condition=Available `
    deployment `
    --all `
    --timeout=15m

$modelTemplate = Get-Content -LiteralPath "$PSScriptRoot\k8s\model-deployment.yaml" -Raw
$modelManifest = $modelTemplate.
    Replace("__MODEL_NAME__", $ModelName).
    Replace("__GPU_POOL_NAME__", $GpuNodePoolName)
$temporaryManifest = Join-Path ([System.IO.Path]::GetTempPath()) "factory-model-$([guid]::NewGuid()).yaml"
try {
    Set-Content -LiteralPath $temporaryManifest -Value $modelManifest -Encoding utf8
    kubectl apply -f $temporaryManifest
}
finally {
    Remove-Item -LiteralPath $temporaryManifest -Force -ErrorAction SilentlyContinue
}

$modelEndpoint = "https://$ModelName.foundry-local-operator.svc.cluster.local:5000/v1/chat/completions"

az k8s-extension create `
    --cluster-type connectedClusters `
    --cluster-name $ClusterName `
    --resource-group $ResourceGroup `
    --name $AgenticRetrievalExtensionName `
    --extension-type microsoft.arc.rag `
    --release-train preview `
    --auto-upgrade false `
    --configuration-settings "layerSelection=$LayerSelection" `
    --configuration-settings isManagedIdentityRequired=true `
    --configuration-settings gpu_enabled=true `
    --configuration-settings AgentOperationTimeoutInMinutes=60 `
    --configuration-settings "auth.tenantId=$TenantId" `
    --configuration-settings "auth.clientId=$EdgeRagClientId" `
    --configuration-settings "ingress.domainname=$DomainName" `
    --configuration-settings byom.enabled=true `
    --configuration-settings "byom.apiEndpoint=$modelEndpoint" `
    --configuration-settings "byom.apiModel=$ModelName" `
    --configuration-settings byom.maxTokensInK=32 `
    --configuration-settings "foundryClientId=$FoundryClientId" `
    --only-show-errors

$clusterScope = az connectedk8s show `
    --resource-group $ResourceGroup `
    --name $ClusterName `
    --query id `
    --output tsv
$connectedClusterPrincipalId = az connectedk8s show `
    --resource-group $ResourceGroup `
    --name $ClusterName `
    --query identity.principalId `
    --output tsv
$foundryPrincipalId = az k8s-extension show `
    --resource-group $ResourceGroup `
    --cluster-name $ClusterName `
    --cluster-type connectedClusters `
    --name inference-operator `
    --query identity.principalId `
    --output tsv
$edgeRagPrincipalId = az k8s-extension show `
    --resource-group $ResourceGroup `
    --cluster-name $ClusterName `
    --cluster-type connectedClusters `
    --name $AgenticRetrievalExtensionName `
    --query identity.principalId `
    --output tsv
$foundryApplicationObjectId = az ad app list `
    --filter "appId eq '$FoundryClientId'" `
    --query "[0].id" `
    --output tsv
$foundryServicePrincipalId = az ad sp list `
    --filter "appId eq '$FoundryClientId'" `
    --query "[0].id" `
    --output tsv
$foundryInferenceRoleId = az rest `
    --method GET `
    --url "https://graph.microsoft.com/v1.0/applications/$foundryApplicationObjectId" `
    --query "appRoles[?value=='FoundryInferenceAccess'] | [0].id" `
    --output tsv

az role assignment create `
    --assignee-object-id $connectedClusterPrincipalId `
    --assignee-principal-type ServicePrincipal `
    --role Reader `
    --scope $clusterScope `
    --only-show-errors

$foundryRoleAssignments = az rest `
    --method GET `
    --url "https://graph.microsoft.com/v1.0/servicePrincipals/$edgeRagPrincipalId/appRoleAssignments" `
    --output json | ConvertFrom-Json
$existingFoundryRoleAssignment = $foundryRoleAssignments.value | Where-Object {
    $_.resourceId -eq $foundryServicePrincipalId -and $_.appRoleId -eq $foundryInferenceRoleId
} | Select-Object -First 1
if (-not $existingFoundryRoleAssignment) {
    $appRoleAssignmentBody = @{
        principalId = $edgeRagPrincipalId
        resourceId = $foundryServicePrincipalId
        appRoleId = $foundryInferenceRoleId
    }
    $temporaryRoleFile = Join-Path ([System.IO.Path]::GetTempPath()) "foundry-role-$([guid]::NewGuid()).json"
    try {
        $appRoleAssignmentBody | ConvertTo-Json | Set-Content -LiteralPath $temporaryRoleFile -Encoding utf8
        az rest `
            --method POST `
            --url "https://graph.microsoft.com/v1.0/servicePrincipals/$edgeRagPrincipalId/appRoleAssignments" `
            --headers "Content-Type=application/json" `
            --body "@$temporaryRoleFile" `
            --output none
        if ($LASTEXITCODE -ne 0) {
            throw "Assigning FoundryInferenceAccess to Agentic Retrieval failed."
        }
    }
    finally {
        Remove-Item -LiteralPath $temporaryRoleFile -Force -ErrorAction SilentlyContinue
    }
}
az role assignment create `
    --assignee-object-id $foundryPrincipalId `
    --assignee-principal-type ServicePrincipal `
    --role Reader `
    --scope $clusterScope `
    --only-show-errors
az role assignment create `
    --assignee-object-id $edgeRagPrincipalId `
    --assignee-principal-type ServicePrincipal `
    --role "Cognitive Services OpenAI User" `
    --scope $clusterScope `
    --only-show-errors
az role assignment create `
    --assignee-object-id $edgeRagPrincipalId `
    --assignee-principal-type ServicePrincipal `
    --role Reader `
    --scope $clusterScope `
    --only-show-errors

Write-Host ""
Write-Host "Cluster bootstrap submitted."
Write-Host "Model endpoint: $modelEndpoint"
Write-Warning "Configure DNS for $DomainName after the ingress load-balancer address is assigned."
