<#
.SYNOPSIS
  Builds the Attune UI and installs it where the daemon embeds it.

.DESCRIPTION
  The daemon compiles daemon/web-content into its binary with include_dir, so
  the built UI has to be on disk before cargo build runs. This script produces
  it from ui/.

  Node is fetched as a portable ZIP into .tools/ rather than installed. The
  Windows installer needs elevation, which a build script should not require.

  Run this after changing anything under ui/, then rebuild the daemon.

.EXAMPLE
  ./scripts/build-ui.ps1
  cargo build --release -p goxlr-daemon
#>

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$repo = Split-Path $PSScriptRoot -Parent
$tools = Join-Path $repo '.tools'
$ui = Join-Path $repo 'ui'
$webContent = Join-Path $repo 'daemon/web-content'

# --- Node ---------------------------------------------------------------

$nodeDir = Get-ChildItem $tools -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -like 'node-*-win-x64' } |
    Select-Object -First 1 -ExpandProperty FullName

if (-not $nodeDir) {
    Write-Host 'Fetching portable Node...'
    New-Item -ItemType Directory -Force -Path $tools | Out-Null

    $lts = (Invoke-RestMethod -Uri 'https://nodejs.org/dist/index.json' |
        Where-Object { $_.lts -ne $false } | Select-Object -First 1).version

    $zip = Join-Path $tools 'node.zip'
    Invoke-WebRequest -Uri "https://nodejs.org/dist/$lts/node-$lts-win-x64.zip" `
        -OutFile $zip -UseBasicParsing
    Expand-Archive -Path $zip -DestinationPath $tools -Force
    Remove-Item $zip -Force

    $nodeDir = Get-ChildItem $tools -Directory |
        Where-Object { $_.Name -like 'node-*-win-x64' } |
        Select-Object -First 1 -ExpandProperty FullName
}

$env:PATH = "$nodeDir;$env:PATH"
Write-Host "Node $(node --version), npm $(npm --version)"

# --- Build --------------------------------------------------------------

Push-Location $ui
try {
    if (-not (Test-Path (Join-Path $ui 'node_modules'))) {
        Write-Host 'Installing UI dependencies...'
        npm install --no-audit --no-fund
        if ($LASTEXITCODE -ne 0) { throw "npm install failed ($LASTEXITCODE)" }
    }

    Write-Host 'Building UI...'
    npm run build
    if ($LASTEXITCODE -ne 0) { throw "npm run build failed ($LASTEXITCODE)" }
}
finally {
    Pop-Location
}

# --- Install ------------------------------------------------------------

$dist = Join-Path $ui 'dist'
if (-not (Test-Path $dist)) { throw "Build produced no dist/ at $dist" }

# Replace rather than merge: Vite hashes asset filenames, so merging would
# leave every previous build's assets behind and grow the binary each time.
if (Test-Path $webContent) { Remove-Item $webContent -Recurse -Force }
New-Item -ItemType Directory -Force -Path $webContent | Out-Null
Copy-Item -Path (Join-Path $dist '*') -Destination $webContent -Recurse -Force

$count = (Get-ChildItem $webContent -Recurse -File).Count
$size = [math]::Round(((Get-ChildItem $webContent -Recurse -File |
    Measure-Object -Property Length -Sum).Sum / 1MB), 2)

Write-Host ""
Write-Host "Installed $count files ($size MB) to daemon/web-content"
Write-Host "Now rebuild the daemon so it embeds them:"
Write-Host "  cargo build --release -p goxlr-daemon"
