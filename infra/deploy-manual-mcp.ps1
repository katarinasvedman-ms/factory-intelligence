[CmdletBinding()]
param(
    [string]$ResourceGroup = $env:FACTORY_RESOURCE_GROUP,
    [string]$ClusterName = $env:FACTORY_CLUSTER_NAME,
    [string]$AcrResourceGroup,
    [string]$AcrName = $env:FACTORY_ACR_NAME,
    [string]$ImageTag = "factory-manual-mcp:3",
    [string]$McpPublicIpName = $env:FACTORY_MCP_PUBLIC_IP_NAME,
    [string]$TenantId,
    [string]$Audience = $env:FACTORY_AGENTIC_CLIENT_ID,
    [string]$AcmeEmail,
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
    AcrName = $AcrName
    McpPublicIpName = $McpPublicIpName
    Audience = $Audience
}.GetEnumerator()) {
    if ([string]::IsNullOrWhiteSpace($entry.Value)) {
        throw "$($entry.Key) is required. Pass it as a parameter or environment variable."
    }
}

if (-not $TenantId) {
    $TenantId = az account show --query tenantId --output tsv
}
if (-not $AcmeEmail) {
    $AcmeEmail = az account show --query user.name --output tsv
}
if (-not $AcrResourceGroup) {
    $AcrResourceGroup = $ResourceGroup
}

$nodeResourceGroup = az aks show `
    --resource-group $ResourceGroup `
    --name $ClusterName `
    --query nodeResourceGroup `
    --output tsv
$mcpIp = az network public-ip show `
    --resource-group $nodeResourceGroup `
    --name $McpPublicIpName `
    --query ipAddress `
    --output tsv
$mcpDomain = "mcp.$($mcpIp.Replace('.', '-')).sslip.io"
$loginServer = az acr show `
    --resource-group $AcrResourceGroup `
    --name $AcrName `
    --query loginServer `
    --output tsv
$image = "$loginServer/$ImageTag"

az aks get-credentials `
    --resource-group $ResourceGroup `
    --name $ClusterName `
    --admin `
    --overwrite-existing `
    --only-show-errors

$imageParts = $ImageTag.Split(":", 2)
$existingTag = az acr repository show-tags `
    --name $AcrName `
    --repository $imageParts[0] `
    --query "[?@=='$($imageParts[1])'] | [0]" `
    --output tsv 2>$null
if ($ForceBuild -or -not $existingTag) {
    az acr build `
        --registry $AcrName `
        --image $ImageTag `
        --file "$PSScriptRoot\mcp\manuals\Dockerfile" `
        "$PSScriptRoot\.." `
        --no-logs `
        --only-show-errors
}

helm repo add ingress-nginx https://kubernetes.github.io/ingress-nginx
helm repo update
helm upgrade --install manual-mcp-ingress ingress-nginx/ingress-nginx `
    --namespace manual-mcp-ingress `
    --create-namespace `
    --set controller.ingressClass=manual-mcp-nginx `
    --set controller.ingressClassResource.name=manual-mcp-nginx `
    --set controller.ingressClassResource.controllerValue=k8s.io/manual-mcp-nginx `
    --set controller.service.loadBalancerIP=$mcpIp `
    --set controller.service.externalTrafficPolicy=Local `
    --set controller.service.annotations."service\.beta\.kubernetes\.io/azure-load-balancer-health-probe-request-path"=/healthz `
    --set controller.nodeSelector.workload=factory-cpu `
    --set controller.admissionWebhooks.patch.nodeSelector.workload=factory-cpu `
    --wait `
    --timeout 10m

$issuer = Get-Content -LiteralPath "$PSScriptRoot\k8s\letsencrypt-cluster-issuer.yaml" -Raw
$issuer = $issuer.Replace("__ACME_EMAIL__", $AcmeEmail)
$issuerPath = Join-Path ([System.IO.Path]::GetTempPath()) "factory-issuer-$([guid]::NewGuid()).yaml"

$manifest = Get-Content -LiteralPath "$PSScriptRoot\k8s\manual-mcp.yaml" -Raw
$manifest = $manifest.
    Replace("__IMAGE__", $image).
    Replace("__TENANT_ID__", $TenantId).
    Replace("__AUDIENCE__", $Audience).
    Replace("__MCP_DOMAIN__", $mcpDomain)
$manifestPath = Join-Path ([System.IO.Path]::GetTempPath()) "factory-mcp-$([guid]::NewGuid()).yaml"

try {
    Set-Content -LiteralPath $issuerPath -Value $issuer -Encoding utf8
    kubectl apply -f $issuerPath
    Set-Content -LiteralPath $manifestPath -Value $manifest -Encoding utf8
    kubectl apply -f $manifestPath
}
finally {
    Remove-Item -LiteralPath $issuerPath, $manifestPath -Force -ErrorAction SilentlyContinue
}

kubectl rollout status deployment/factory-manual-mcp `
    --namespace factory-mcp `
    --timeout 10m
kubectl wait certificate/factory-manual-mcp `
    --namespace factory-mcp `
    --for=condition=Ready `
    --timeout 10m

[pscustomobject]@{
    Image = $image
    Domain = $mcpDomain
    Endpoint = "https://$mcpDomain/mcp"
} | ConvertTo-Json
