[CmdletBinding()]
param(
    [string]$ResourceGroup = $env:FACTORY_RESOURCE_GROUP,
    [string]$Location = $env:FACTORY_LOCATION,
    [string]$NamePrefix = $env:FACTORY_NAME_PREFIX,
    [string]$SshPublicKeyPath = "$HOME\.ssh\id_ed25519.pub",
    [switch]$SkipGpuPool
)

$ErrorActionPreference = "Stop"

foreach ($entry in @{
    ResourceGroup = $ResourceGroup
    Location = $Location
    NamePrefix = $NamePrefix
}.GetEnumerator()) {
    if ([string]::IsNullOrWhiteSpace($entry.Value)) {
        throw "$($entry.Key) is required. Pass it as a parameter or environment variable."
    }
}

if (-not (Get-Command az -ErrorAction SilentlyContinue)) {
    throw "Azure CLI is required."
}

if (-not (Test-Path -LiteralPath $SshPublicKeyPath)) {
    $privateKeyPath = Join-Path `
        (Split-Path $SshPublicKeyPath) `
        ([System.IO.Path]::GetFileNameWithoutExtension($SshPublicKeyPath))
    New-Item -ItemType Directory -Path (Split-Path $SshPublicKeyPath) -Force | Out-Null
    if (-not (Test-Path -LiteralPath $privateKeyPath)) {
        ssh-keygen -t ed25519 -f $privateKeyPath -N ""
    }
    elseif (Test-Path -LiteralPath "$privateKeyPath..pub") {
        Move-Item -LiteralPath "$privateKeyPath..pub" -Destination $SshPublicKeyPath
    }
}

$env:AKS_SSH_PUBLIC_KEY = (Get-Content -LiteralPath $SshPublicKeyPath -Raw).Trim()

az account show --output none
az group show --name $ResourceGroup --output none
az bicep build --file "$PSScriptRoot\main.bicep" --stdout | Out-Null

$parameters = @{
    location = $Location
    namePrefix = $NamePrefix
    deployGpuPool = -not $SkipGpuPool.IsPresent
}

$parameterArguments = @()
foreach ($entry in $parameters.GetEnumerator()) {
    $parameterArguments += "$($entry.Key)=$($entry.Value.ToString().ToLowerInvariant())"
}

az stack group create `
    --name "factory-intelligence-demo" `
    --resource-group $ResourceGroup `
    --template-file "$PSScriptRoot\main.bicep" `
    --parameters "$PSScriptRoot\main.bicepparam" `
    --parameters $parameterArguments `
    --action-on-unmanage deleteResources `
    --deny-settings-mode None `
    --yes `
    --output table
