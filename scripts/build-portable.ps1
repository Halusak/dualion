$ErrorActionPreference = 'Stop'

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
  throw 'The portable Windows build script must be run on Windows.'
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$package = Get-Content -LiteralPath (Join-Path $repoRoot 'package.json') -Raw | ConvertFrom-Json
$binary = Join-Path $repoRoot 'src-tauri\target\release\Dualion.exe'
$portableDir = Join-Path $repoRoot 'portable'

Push-Location $repoRoot
try {
  if (-not (Test-Path -LiteralPath (Join-Path $repoRoot 'node_modules'))) {
    throw 'Dependencies are missing. Run npm ci first.'
  }

  Write-Host "Building Dualion community fork $($package.version) without installer bundling..."
  & npm.cmd run tauri -- build --no-bundle
  if ($LASTEXITCODE -ne 0) {
    throw "Tauri build failed with exit code $LASTEXITCODE."
  }

  if (-not (Test-Path -LiteralPath $binary)) {
    throw "Build completed, but the executable was not found: $binary"
  }

  New-Item -ItemType Directory -Path $portableDir -Force | Out-Null
  Copy-Item -LiteralPath $binary -Destination (Join-Path $portableDir 'Dualion.exe') -Force
  $marker = Join-Path $portableDir 'portable.dat'
  if (-not (Test-Path -LiteralPath $marker)) {
    New-Item -ItemType File -Path $marker | Out-Null
  }

  Write-Host "Portable build ready: $portableDir"
  Write-Host 'Run portable\Dualion.exe. Existing data and settings in the folder are preserved.'
}
finally {
  Pop-Location
}
