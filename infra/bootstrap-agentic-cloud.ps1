[CmdletBinding()]
param(
    [string]$ResourceGroup = $env:FACTORY_RESOURCE_GROUP,
    [string]$ClusterName = $env:FACTORY_CLUSTER_NAME,
    [string]$FoundryResourceGroup,
    [string]$FoundryAccountName = $env:FACTORY_FOUNDRY_ACCOUNT_NAME,
    [string]$FoundryDeploymentName = $env:FOUNDRY_SLOW_MODEL,
    [string]$AcrResourceGroup,
    [string]$AcrName = $env:FACTORY_ACR_NAME,
    [string]$BridgeImageTag = "factory-foundry-bridge:5",
    [string]$BridgeIdentityName = $env:FACTORY_BRIDGE_IDENTITY_NAME,
    [string]$EdgeRagClientId = $env:FACTORY_AGENTIC_CLIENT_ID,
    [string]$AgenticPublicIpName = $env:FACTORY_AGENTIC_PUBLIC_IP_NAME,
    [string]$ExtensionName = "edgeragdemo",
    [string]$ExtensionVersion,
    [ValidateSet("stable", "preview", "dev")]
    [string]$ReleaseTrain = "preview",
    [string]$TenantId,
    [switch]$ForceBuild
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true
$env:PYTHONUTF8 = "1"
$env:PYTHONIOENCODING = "utf-8"
[Console]::OutputEncoding = [Text.UTF8Encoding]::new()

foreach ($entry in @{
    ResourceGroup = $ResourceGroup
    ClusterName = $ClusterName
    FoundryAccountName = $FoundryAccountName
    FoundryDeploymentName = $FoundryDeploymentName
    AcrName = $AcrName
    BridgeIdentityName = $BridgeIdentityName
    EdgeRagClientId = $EdgeRagClientId
    AgenticPublicIpName = $AgenticPublicIpName
}.GetEnumerator()) {
    if ([string]::IsNullOrWhiteSpace($entry.Value)) {
        throw "$($entry.Key) is required. Pass it as a parameter or environment variable."
    }
}

if (-not $TenantId) {
    $TenantId = az account show --query tenantId --output tsv
}
if (-not $FoundryResourceGroup) {
    $FoundryResourceGroup = $ResourceGroup
}
if (-not $AcrResourceGroup) {
    $AcrResourceGroup = $ResourceGroup
}

$nodeResourceGroup = az aks show `
    --resource-group $ResourceGroup `
    --name $ClusterName `
    --query nodeResourceGroup `
    --output tsv
$agenticIp = az network public-ip show `
    --resource-group $nodeResourceGroup `
    --name $AgenticPublicIpName `
    --query ipAddress `
    --output tsv
$domainName = "agentic.$($agenticIp.Replace('.', '-')).sslip.io"
$appObjectId = az ad app show --id $EdgeRagClientId --query id --output tsv
$servicePrincipalId = az ad sp show --id $EdgeRagClientId --query id --output tsv
$userId = az ad signed-in-user show --query id --output tsv
$developerRoleId = az ad app show `
    --id $EdgeRagClientId `
    --query "appRoles[?value=='EdgeRAGDeveloper'] | [0].id" `
    --output tsv

az ad app update `
    --id $EdgeRagClientId `
    --identifier-uris "api://$EdgeRagClientId" `
    --web-redirect-uris "https://$domainName" "https://$domainName/oauth2/callback"
az rest `
    --method PATCH `
    --url "https://graph.microsoft.com/v1.0/applications/$appObjectId" `
    --headers "Content-Type=application/json" `
    --body '{"api":{"requestedAccessTokenVersion":2}}' `
    --output none

$assignments = az rest `
    --method GET `
    --url "https://graph.microsoft.com/v1.0/servicePrincipals/$servicePrincipalId/appRoleAssignedTo" `
    --output json | ConvertFrom-Json
$developerAssignment = $assignments.value | Where-Object {
    $_.principalId -eq $userId -and $_.appRoleId -eq $developerRoleId
} | Select-Object -First 1
if (-not $developerAssignment) {
    $assignment = @{
        principalId = $userId
        resourceId = $servicePrincipalId
        appRoleId = $developerRoleId
    } | ConvertTo-Json
    az rest `
        --method POST `
        --url "https://graph.microsoft.com/v1.0/servicePrincipals/$servicePrincipalId/appRoleAssignedTo" `
        --headers "Content-Type=application/json" `
        --body $assignment `
        --output none
}

$foundryResourceId = az cognitiveservices account show `
    --resource-group $FoundryResourceGroup `
    --name $FoundryAccountName `
    --query id `
    --output tsv
$foundryEndpoint = "https://$FoundryAccountName.openai.azure.com"
$oidcIssuer = az aks show `
    --resource-group $ResourceGroup `
    --name $ClusterName `
    --query oidcIssuerProfile.issuerUrl `
    --output tsv

$identity = az identity list `
    --resource-group $ResourceGroup `
    --query "[?name=='$BridgeIdentityName'] | [0]" `
    --output json | ConvertFrom-Json
if (-not $identity) {
    $identity = az identity create `
        --resource-group $ResourceGroup `
        --name $BridgeIdentityName `
        --location "$(az aks show -g $ResourceGroup -n $ClusterName --query location -o tsv)" `
        --output json | ConvertFrom-Json
}

$federatedCredential = az identity federated-credential list `
    --resource-group $ResourceGroup `
    --identity-name $BridgeIdentityName `
    --query "[?name=='foundry-bridge'] | [0]" `
    --output json 2>$null | ConvertFrom-Json
if (-not $federatedCredential) {
    az identity federated-credential create `
        --resource-group $ResourceGroup `
        --identity-name $BridgeIdentityName `
        --name foundry-bridge `
        --issuer $oidcIssuer `
        --subject "system:serviceaccount:factory-model-bridge:foundry-bridge" `
        --audiences "api://AzureADTokenExchange" `
        --output none
}

$roleAssignment = az role assignment list `
    --assignee-object-id $identity.principalId `
    --scope $foundryResourceId `
    --role "Cognitive Services OpenAI User" `
    --query "[0].id" `
    --output tsv
if (-not $roleAssignment) {
    az role assignment create `
        --assignee-object-id $identity.principalId `
        --assignee-principal-type ServicePrincipal `
        --role "Cognitive Services OpenAI User" `
        --scope $foundryResourceId `
        --output none
}

$loginServer = az acr show `
    --resource-group $AcrResourceGroup `
    --name $AcrName `
    --query loginServer `
    --output tsv
$bridgeImage = "$loginServer/$BridgeImageTag"
$imageParts = $BridgeImageTag.Split(":", 2)
$existingTag = az acr repository show-tags `
    --name $AcrName `
    --repository $imageParts[0] `
    --query "[?@=='$($imageParts[1])'] | [0]" `
    --output tsv 2>$null
if ($ForceBuild -or -not $existingTag) {
    az acr build `
        --registry $AcrName `
        --image $BridgeImageTag `
        --file "$PSScriptRoot\model-bridge\Dockerfile" `
        "$PSScriptRoot\.." `
        --no-logs `
        --only-show-errors
}

kubectl create namespace factory-model-bridge --dry-run=client -o yaml |
    kubectl apply -f -
$encodedBridgeSecret = kubectl get secret foundry-bridge-auth `
    --namespace factory-model-bridge `
    --ignore-not-found `
    --output jsonpath="{.data.BRIDGE_SHARED_SECRET}" 2>$null
$bridgeSecret = if ($encodedBridgeSecret) {
    [Text.Encoding]::UTF8.GetString(
        [Convert]::FromBase64String($encodedBridgeSecret)
    )
}
else {
    [Convert]::ToBase64String(
        [Security.Cryptography.RandomNumberGenerator]::GetBytes(48)
    )
}
$bridgeManifest = Get-Content `
    -LiteralPath "$PSScriptRoot\k8s\foundry-model-bridge.yaml" `
    -Raw
$bridgeManifest = $bridgeManifest.
    Replace("__MANAGED_IDENTITY_CLIENT_ID__", $identity.clientId).
    Replace("__BRIDGE_SHARED_SECRET__", $bridgeSecret).
    Replace("__IMAGE__", $bridgeImage).
    Replace("__FOUNDRY_ENDPOINT__", $foundryEndpoint)
$bridgeManifestPath = Join-Path `
    ([System.IO.Path]::GetTempPath()) `
    "factory-bridge-$([guid]::NewGuid()).yaml"
try {
    Set-Content `
        -LiteralPath $bridgeManifestPath `
        -Value $bridgeManifest `
        -Encoding utf8
    kubectl apply -f $bridgeManifestPath
}
finally {
    Remove-Item -LiteralPath $bridgeManifestPath -Force -ErrorAction SilentlyContinue
}
kubectl rollout status deployment/foundry-model-bridge `
    --namespace factory-model-bridge `
    --timeout 10m

kubectl create namespace arc-rag --dry-run=client -o yaml | kubectl apply -f -
kubectl delete secret byom-api-key --namespace arc-rag --ignore-not-found
kubectl create secret generic byom-api-key `
    --namespace arc-rag `
    --from-literal=BYOM_API_KEY=$bridgeSecret

$apiEndpoint = "http://foundry-model-bridge.factory-model-bridge.svc.cluster.local:8000/openai/deployments/$FoundryDeploymentName/chat/completions?api-version=2024-10-21"

$existingExtension = az k8s-extension list `
    --cluster-type connectedClusters `
    --cluster-name $ClusterName `
    --resource-group $ResourceGroup `
    --query "[?name=='$ExtensionName'] | [0]" `
    --output json | ConvertFrom-Json
if ($existingExtension -and $existingExtension.provisioningState -eq "Failed") {
    az k8s-extension delete `
        --cluster-type connectedClusters `
        --cluster-name $ClusterName `
        --resource-group $ResourceGroup `
        --name $ExtensionName `
        --yes `
        --only-show-errors
}

$extensionArguments = @(
    "k8s-extension", "create",
    "--cluster-type", "connectedClusters",
    "--cluster-name", $ClusterName,
    "--resource-group", $ResourceGroup,
    "--name", $ExtensionName,
    "--extension-type", "microsoft.arc.rag",
    "--release-train", $ReleaseTrain,
    "--auto-upgrade", "false",
    "--configuration-settings", "isManagedIdentityRequired=true",
    "--configuration-settings", "gpu_enabled=false",
    "--configuration-settings", "AgentOperationTimeoutInMinutes=60",
    "--configuration-settings", "auth.tenantId=$TenantId",
    "--configuration-settings", "auth.clientId=$EdgeRagClientId",
    "--configuration-settings", "ingress.domainname=$domainName",
    "--configuration-settings", "layerSelection=agentic",
    "--configuration-settings", "byom.enabled=true",
    "--configuration-settings", "byom.apiEndpoint=$apiEndpoint",
    "--configuration-settings", "byom.apiModel=$FoundryDeploymentName",
    "--configuration-settings", "byom.maxTokensInK=128",
    "--configuration-settings", "agenticChat.image.repository=mcr.microsoft.com/azurearcai/apps/edge-core-chat-frontend",
    "--configuration-settings", "ingress-nginx.controller.config.strict-validate-path-type=false",
    "--configuration-settings", "ingress-nginx.controller.service.loadBalancerIP=$agenticIp",
    "--configuration-settings", "ingress-nginx.controller.service.annotations.service\.beta\.kubernetes\.io/azure-load-balancer-health-probe-request-path=/healthz",
    "--only-show-errors"
)
if ($ExtensionVersion) {
    $extensionArguments += @("--version", $ExtensionVersion)
}
az @extensionArguments

$certificateManifest = Get-Content `
    -LiteralPath "$PSScriptRoot\k8s\agentic-certificate.yaml" `
    -Raw
$certificateManifest = $certificateManifest.Replace(
    "__AGENTIC_DOMAIN__",
    $domainName
)
$certificateManifestPath = Join-Path `
    ([System.IO.Path]::GetTempPath()) `
    "factory-agentic-certificate-$([guid]::NewGuid()).yaml"
try {
    Set-Content `
        -LiteralPath $certificateManifestPath `
        -Value $certificateManifest `
        -Encoding utf8
    kubectl apply -f $certificateManifestPath
}
finally {
    Remove-Item `
        -LiteralPath $certificateManifestPath `
        -Force `
        -ErrorAction SilentlyContinue
}
kubectl wait certificate/agentic-ingress `
    --namespace arc-rag `
    --for=condition=Ready `
    --timeout 10m

[pscustomobject]@{
    Domain = $domainName
    BaseUrl = "https://$domainName"
    Extension = $ExtensionName
    ModelEndpoint = $apiEndpoint
    Authentication = "AKS Workload Identity to keyless Microsoft Foundry"
} | ConvertTo-Json
