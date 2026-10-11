[CmdletBinding()]
param(
    [string]$InnoCompiler,
    [string]$TestRoot
)

$ErrorActionPreference = 'Stop'
$installerRepoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$manifestText = Get-Content -LiteralPath (Join-Path $installerRepoRoot 'Cargo.toml') -Raw
$packageMatch = [regex]::Match($manifestText, '(?ms)^\[package\]\s*\r?\n(?<package>.*?)(?=^\[|\z)')
if (-not $packageMatch.Success) { throw 'Cannot find the [package] section in Cargo.toml.' }
$versionMatch = [regex]::Match($packageMatch.Groups['package'].Value, '(?m)^\s*version\s*=\s*"(?<version>[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?)"\s*$')
if (-not $versionMatch.Success) { throw 'Cannot read the package version from Cargo.toml.' }
$installerVersion = $versionMatch.Groups['version'].Value
$numericVersion = [regex]::Match($installerVersion, '^[0-9]+\.[0-9]+\.[0-9]+').Value + '.0'

foreach ($relativePath in @(
    'target\bundled\ShapeRack.vst3\Contents\x86_64-win\ShapeRack.vst3',
    'target\bundled\ShapeRack.clap',
    'installer\ShapeRack.iss',
    'installer\AfterInstall.txt',
    'README.md', 'INSTALL.md', 'VALIDATION.md', 'THIRD_PARTY.md', 'LICENSE',
    'licenses\NIH-plug-ISC.txt', 'licenses\DEPENDENCY-NOTICES.txt'
)) {
    if (-not (Test-Path -LiteralPath (Join-Path $installerRepoRoot $relativePath) -PathType Leaf)) {
        throw "Required input is missing: $relativePath. Build plugins with cargo xtask bundle shaperack --release --locked before compiling the installer."
    }
}

if (-not $InnoCompiler) {
    $pathCompiler = Get-Command 'ISCC.exe' -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($pathCompiler) {
        $InnoCompiler = $pathCompiler.Source
    } else {
        foreach ($programFilesRoot in @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
            if ($programFilesRoot) {
                $candidateCompiler = Join-Path $programFilesRoot 'Inno Setup 6\ISCC.exe'
                if (Test-Path -LiteralPath $candidateCompiler -PathType Leaf) {
                    $InnoCompiler = $candidateCompiler
                    break
                }
            }
        }
    }
}
if (-not $InnoCompiler -or -not (Test-Path -LiteralPath $InnoCompiler -PathType Leaf)) {
    throw 'Inno Setup compiler not found. Install Inno Setup 6.3 or newer, put ISCC.exe on PATH, or pass -InnoCompiler with its full path.'
}
$InnoCompiler = (Resolve-Path -LiteralPath $InnoCompiler).Path

$compilerArguments = @(
    '/Qp',
    "/DRepoRoot=$installerRepoRoot",
    "/DAppVersion=$installerVersion",
    "/DFileVersion=$numericVersion"
)
$installerOutput = Join-Path $installerRepoRoot 'dist'
$installerBaseName = "ShapeRack-$installerVersion-Setup"
if ($TestRoot) {
    if (-not [System.IO.Path]::IsPathRooted($TestRoot)) {
        throw '-TestRoot must be an absolute staging directory.'
    }
    $installerTestRoot = [System.IO.Path]::GetFullPath($TestRoot).TrimEnd('\', '/')
    $driveRoot = [System.IO.Path]::GetPathRoot($installerTestRoot).TrimEnd('\', '/')
    if ($installerTestRoot -eq $driveRoot) {
        throw '-TestRoot must name a staging directory, not a drive or share root.'
    }
    # These characters could be interpreted as Inno constants or script syntax.
    if ($installerTestRoot -match '[{}"''\r\n]') {
        throw '-TestRoot cannot contain braces, quotes, or line breaks.'
    }
    $compilerArguments += "/DTestRoot=$installerTestRoot"
    $installerOutput = Join-Path $installerOutput 'test-installer'
    $installerBaseName += '-TEST'
    Write-Host "Building a staging-only installer for: $installerTestRoot"
}
New-Item -ItemType Directory -Path $installerOutput -Force | Out-Null
$compilerArguments += (Join-Path $installerRepoRoot 'installer\ShapeRack.iss')
& $InnoCompiler @compilerArguments
if ($LASTEXITCODE -ne 0) { throw "Inno Setup compilation failed with exit code $LASTEXITCODE." }
$installerPath = Join-Path $installerOutput "$installerBaseName.exe"
if (-not (Test-Path -LiteralPath $installerPath -PathType Leaf)) {
    throw "Compilation finished but the installer was not found: $installerPath"
}
Get-Item -LiteralPath $installerPath | Select-Object FullName, Length
