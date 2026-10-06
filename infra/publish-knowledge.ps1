[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$NfsMountPath
)

$ErrorActionPreference = "Stop"

$source = Resolve-Path (Join-Path $PSScriptRoot "..\data\knowledge\source")
if (-not (Test-Path -LiteralPath $NfsMountPath -PathType Container)) {
    throw "NFS mount path does not exist: $NfsMountPath"
}

$destination = Join-Path $NfsMountPath "factory-maintenance-demo"
New-Item -ItemType Directory -Path $destination -Force | Out-Null
Copy-Item -Path (Join-Path $source "*") -Destination $destination -Force

$files = Get-ChildItem -LiteralPath $destination -File |
    Select-Object Name, Length, LastWriteTimeUtc

Write-Host "Published synthetic factory knowledge to $destination"
$files | Format-Table -AutoSize
