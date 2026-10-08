[CmdletBinding()]
param(
    [string]$ResourceGroup = $env:FACTORY_RESOURCE_GROUP,
    [string]$ClusterName = $env:FACTORY_CLUSTER_NAME,
    [string]$AcrName = $env:FACTORY_ACR_NAME,
    [string]$ImageTag = "factory-advisory-worker:1",
    [string]$FactoryId = "factory-demo-01",
    [string]$EdgeId = "edge-demo-01",
    [string]$AgenticDomain = $env:FACTORY_AGENTIC_DOMAIN,
    [string]$AgenticClientId = $env:FACTORY_AGENTIC_CLIENT_ID,
    [string]$MqttUsername = "factory-demo",
    [string]$MqttPassword,
    [string]$LocalConfigPath = ".env.local",
    [switch]$ForceBuild
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true

function Import-LocalEnvironment {
    param([string]$Path)
    $resolved = if ([IO.Path]::IsPathRooted($Path)) { $Path } else { Join-Path (Split-Path $PSScriptRoot -Parent) $Path }
    if (-not (Test-Path -LiteralPath $resolved)) { return $resolved }
    foreach ($line in Get-Content -LiteralPath $resolved) {
        $trimmed = $line.Trim()
        if (-not $trimmed -or $trimmed.StartsWith("#") -or -not $trimmed.Contains("=")) { continue }
        $name, $value = $trimmed.Split("=", 2)
        if (-not [Environment]::GetEnvironmentVariable($name, "Process")) {
            [Environment]::SetEnvironmentVariable($name, $value.Trim(), "Process")
        }
    }
    return $resolved
}

function Set-LocalEnvironmentValue {
    param([string]$Path, [string]$Name, [string]$Value)
    $lines = [Collections.Generic.List[string]]::new()
    if (Test-Path -LiteralPath $Path) {
        foreach ($line in Get-Content -LiteralPath $Path) {
            $lines.Add($line)
        }
    }
    $prefix = "$Name="
    $index = -1
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i].StartsWith($prefix, [StringComparison]::Ordinal)) { $index = $i; break }
    }
    if ($index -ge 0) { $lines[$index] = "$prefix$Value" } else { $lines.Add("$prefix$Value") }
    Set-Content -LiteralPath $Path -Value $lines -Encoding utf8
}

$localEnvironment = Import-LocalEnvironment -Path $LocalConfigPath
if (-not $ResourceGroup) { $ResourceGroup = $env:FACTORY_RESOURCE_GROUP }
if (-not $ClusterName) { $ClusterName = $env:FACTORY_CLUSTER_NAME }
if (-not $AcrName) { $AcrName = $env:FACTORY_ACR_NAME }
if (-not $AgenticDomain) { $AgenticDomain = $env:FACTORY_AGENTIC_DOMAIN }
if (-not $AgenticClientId) { $AgenticClientId = $env:FACTORY_AGENTIC_CLIENT_ID }
if (-not $MqttPassword) { $MqttPassword = $env:MQTT_PASSWORD }
foreach ($entry in @{
    ResourceGroup = $ResourceGroup
    ClusterName = $ClusterName
    AcrName = $AcrName
    AgenticDomain = $AgenticDomain
    AgenticClientId = $AgenticClientId
}.GetEnumerator()) {
    if (-not $entry.Value) { throw "$($entry.Key) is required." }
}

if (-not $MqttPassword) {
    $bytes = New-Object byte[] 24
    [Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
    $MqttPassword = [Convert]::ToBase64String($bytes)
}

az aks get-credentials --resource-group $ResourceGroup --name $ClusterName --admin --overwrite-existing --only-show-errors
$loginServer = az acr show --name $AcrName --query loginServer --output tsv
$image = "$loginServer/$ImageTag"
$imageParts = $ImageTag.Split(":", 2)
$existingTag = $null
if (-not $ForceBuild) {
    $existingTag = az acr repository show-tags `
        --name $AcrName `
        --repository $imageParts[0] `
        --output tsv 2>$null |
        Where-Object { $_ -eq $imageParts[1] } |
        Select-Object -First 1
}
if ($ForceBuild -or -not $existingTag) {
    az acr build --registry $AcrName --image $ImageTag --file "$PSScriptRoot\advisory-worker\Dockerfile" "$PSScriptRoot\.." --no-logs --only-show-errors
}

$agenticToken = (az account get-access-token --resource "api://$AgenticClientId" --query accessToken --output tsv).Trim()
if (-not $agenticToken) { throw "Unable to acquire the Agentic Retrieval token." }
$agenticBaseUrl = "https://$AgenticDomain"
$knowledgeBases = $null
for ($attempt = 1; $attempt -le 5; $attempt++) {
    try {
        $knowledgeBases = Invoke-RestMethod `
            -Uri "$agenticBaseUrl/knowledge-bases?limit=1" `
            -Headers @{ Authorization = "Bearer $agenticToken" } `
            -TimeoutSec 60
        break
    }
    catch {
        if ($attempt -eq 5) { throw }
        Start-Sleep -Seconds (3 * $attempt)
    }
}
$agentId = @($knowledgeBases.data)[0].id
if (-not $agentId) { throw "Agentic Retrieval returned no knowledge base." }

kubectl create namespace factory-messaging --dry-run=client -o yaml | kubectl apply -f -
kubectl create secret generic factory-mqtt-credentials `
    --namespace factory-messaging `
    --from-literal=username=$MqttUsername `
    --from-literal=password=$MqttPassword `
    --dry-run=client -o yaml | kubectl apply -f -
kubectl create secret generic factory-advisory-agentic `
    --namespace factory-messaging `
    --from-literal=token=$agenticToken `
    --dry-run=client -o yaml | kubectl apply -f -

kubectl apply -f "$PSScriptRoot\k8s\factory-mqtt.yaml"
$requestTopic = "factory/v1/$FactoryId/advisory/requests"
$responseTopic = "factory/v1/$FactoryId/edges/$EdgeId/advisory/responses"
$manifest = Get-Content -LiteralPath "$PSScriptRoot\k8s\advisory-worker.yaml" -Raw
$manifest = $manifest.
    Replace("__IMAGE__", $image).
    Replace("__FACTORY_ID__", $FactoryId).
    Replace("__EDGE_ID__", $EdgeId).
    Replace("__REQUEST_TOPIC__", $requestTopic).
    Replace("__AGENTIC_BASE_URL__", $agenticBaseUrl).
    Replace("__AGENT_ID__", $agentId)
$manifestPath = Join-Path ([IO.Path]::GetTempPath()) "factory-advisory-worker-$([guid]::NewGuid()).yaml"
try {
    Set-Content -LiteralPath $manifestPath -Value $manifest -Encoding utf8
    kubectl apply -f $manifestPath
    kubectl rollout restart deployment/factory-advisory-worker --namespace factory-messaging
}
finally {
    Remove-Item -LiteralPath $manifestPath -Force -ErrorAction SilentlyContinue
}

kubectl rollout status statefulset/factory-mqtt --namespace factory-messaging --timeout 10m
kubectl rollout status deployment/factory-advisory-worker --namespace factory-messaging --timeout 10m
$deadline = [DateTime]::UtcNow.AddMinutes(10)
do {
    $mqttHost = kubectl get service factory-mqtt --namespace factory-messaging --output jsonpath='{.status.loadBalancer.ingress[0].ip}'
    if (-not $mqttHost) {
        $mqttHost = kubectl get service factory-mqtt --namespace factory-messaging --output jsonpath='{.status.loadBalancer.ingress[0].hostname}'
    }
    if ($mqttHost) { break }
    Start-Sleep -Seconds 10
} while ([DateTime]::UtcNow -lt $deadline)
if (-not $mqttHost) { throw "The MQTT broker did not receive an external address." }

Set-LocalEnvironmentValue $localEnvironment "ADVISORY_TRANSPORT" "mqtt"
Set-LocalEnvironmentValue $localEnvironment "EDGE_ID" $EdgeId
Set-LocalEnvironmentValue $localEnvironment "MQTT_HOST" $mqttHost
Set-LocalEnvironmentValue $localEnvironment "MQTT_PORT" "1883"
Set-LocalEnvironmentValue $localEnvironment "MQTT_TLS" "false"
Set-LocalEnvironmentValue $localEnvironment "MQTT_CLIENT_ID" "governed-floor-$EdgeId"
Set-LocalEnvironmentValue $localEnvironment "MQTT_USERNAME" $MqttUsername
Set-LocalEnvironmentValue $localEnvironment "MQTT_PASSWORD" $MqttPassword
Set-LocalEnvironmentValue $localEnvironment "MQTT_REQUEST_TOPIC" $requestTopic
Set-LocalEnvironmentValue $localEnvironment "MQTT_RESPONSE_TOPIC" $responseTopic

[pscustomobject]@{
    Image = $image
    BrokerHost = $mqttHost
    BrokerPort = 1883
    RequestTopic = $requestTopic
    ResponseTopic = $responseTopic
    Worker = "factory-messaging/factory-advisory-worker"
    LocalConfiguration = $localEnvironment
} | ConvertTo-Json
