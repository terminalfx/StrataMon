<#
.SYNOPSIS
    Builds StrataMon as one file with nothing else to install, at dist\StrataMon-win-x64.exe.
    The Linux file is built by GitHub Actions. See .github\workflows\release.yml.
#>
[CmdletBinding()]
param(
    [string]$Output
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent

# Set here, because Windows PowerShell leaves $PSScriptRoot empty in param defaults.
if (-not $Output) {
    $Output = Join-Path $root 'dist'
}

$target = Join-Path $Output 'StrataMon-win-x64.exe'

# Windows won't overwrite a running exe, and stopping someone's monitor isn't this script's call.
$running = Get-CimInstance Win32_Process -Filter "Name='StrataMon-win-x64.exe'" | Where-Object { $_.ExecutablePath -eq $target }
if ($running) {
    throw "StrataMon is running from $target. Close it, then run this again."
}

& cargo build --release --manifest-path (Join-Path $root 'Cargo.toml') -p stratamon
if ($LASTEXITCODE -ne 0) {
    throw 'The build failed.'
}

New-Item -ItemType Directory -Force $Output | Out-Null
Copy-Item (Join-Path $root 'target\release\StrataMon.exe') $target -Force

$size = (Get-Item $target).Length / 1MB
Write-Host ('Done: {0} ({1:N1} MB)' -f $target, $size)
