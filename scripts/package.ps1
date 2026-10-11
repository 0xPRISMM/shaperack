param([string]$Version = '0.1.0')
$ErrorActionPreference = 'Stop'
$repoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$bundleRoot = Join-Path $repoRoot 'target\bundled'
$stageRoot = Join-Path $repoRoot "dist\ShapeRack-$Version-windows-x64"
$zipPath = Join-Path $repoRoot "dist\ShapeRack-$Version-windows-x64.zip"
if (-not (Test-Path -LiteralPath (Join-Path $bundleRoot 'ShapeRack.vst3'))) { throw 'VST3 bundle missing. Run cargo xtask bundle shaperack --release --locked first.' }
if (-not (Test-Path -LiteralPath (Join-Path $bundleRoot 'ShapeRack.clap'))) { throw 'CLAP bundle missing.' }
New-Item -ItemType Directory -Force -Path $stageRoot | Out-Null
Copy-Item -LiteralPath (Join-Path $bundleRoot 'ShapeRack.vst3') -Destination $stageRoot -Recurse -Force
Copy-Item -LiteralPath (Join-Path $bundleRoot 'ShapeRack.clap') -Destination $stageRoot -Force
foreach ($name in @('README.md', 'INSTALL.md', 'LICENSE', 'THIRD_PARTY.md', 'VALIDATION.md')) {
    Copy-Item -LiteralPath (Join-Path $repoRoot $name) -Destination $stageRoot -Force
}
Copy-Item -LiteralPath (Join-Path $repoRoot 'licenses') -Destination $stageRoot -Recurse -Force
Compress-Archive -Path (Join-Path $stageRoot '*') -DestinationPath $zipPath -Force
Get-Item -LiteralPath $zipPath | Select-Object FullName,Length
