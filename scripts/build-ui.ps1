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
#
# Order matters. A previously fetched portable Node wins so a machine that has
# one keeps building against the same one. Otherwise an already-installed Node
# is used -- a CI runner has one, and downloading a second is a network round
# trip that can fail, which is how the first attune-v1.0.0 release build died.
# Fetching is the last resort, for a developer with no Node at all.

$nodeDir = Get-ChildItem $tools -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -like 'node-*-win-x64' } |
    Select-Object -First 1 -ExpandProperty FullName

if (-not $nodeDir -and (Get-Command node -ErrorAction SilentlyContinue)) {
    Write-Host 'Using the Node already on PATH.'
}
elseif (-not $nodeDir) {
    Write-Host 'Fetching portable Node...'
    New-Item -ItemType Directory -Force -Path $tools | Out-Null

    # Every release carries an `lts` field: the codename once it is an LTS,
    # and JSON false until then. Match the codename explicitly rather than
    # testing `-ne $false`, which compares a string against a bool and leans
    # on coercion to do the right thing.
    #
    # The release build that died here got a 400, which is what this URL
    # returns when the version interpolates to nothing. Why it was empty on
    # the runner is NOT established: `-ne $false` was the obvious suspect and
    # it does not reproduce -- checked against pwsh 7.7, where old and new
    # selectors both return the same version out of 287 matches. So the throw
    # below matters more than the selector does. It turns an unresolvable
    # version into a message that says so, instead of a malformed URL.
    $index = Invoke-RestMethod -Uri 'https://nodejs.org/dist/index.json'
    $lts = ($index | Where-Object { $_.lts -is [string] -and $_.lts } |
        Select-Object -First 1).version
    if (-not $lts) { throw 'Could not determine the current Node LTS version' }

    $zip = Join-Path $tools 'node.zip'
    Invoke-WebRequest -Uri "https://nodejs.org/dist/$lts/node-$lts-win-x64.zip" `
        -OutFile $zip -UseBasicParsing
    Expand-Archive -Path $zip -DestinationPath $tools -Force
    Remove-Item $zip -Force

    $nodeDir = Get-ChildItem $tools -Directory |
        Where-Object { $_.Name -like 'node-*-win-x64' } |
        Select-Object -First 1 -ExpandProperty FullName
}

if ($nodeDir) { $env:PATH = "$nodeDir;$env:PATH" }
if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    throw 'No node on PATH after resolution'
}
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
