[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$InnoCompiler,
    [string]$Python = 'python',
    [string]$TestRoot
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0
$validationRepoRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $TestRoot) {
    $TestRoot = Join-Path $validationRepoRoot 'target\installer-test'
}
if (-not [System.IO.Path]::IsPathRooted($TestRoot)) {
    throw '-TestRoot must be an absolute staging directory.'
}
$validationRoot = [System.IO.Path]::GetFullPath($TestRoot).TrimEnd('\', '/')
$validationDrive = [System.IO.Path]::GetPathRoot($validationRoot).TrimEnd('\', '/')
if ($validationRoot -eq $validationDrive -or $validationRoot -match '[{}"''\r\n]') {
    throw 'Use a staging directory below a drive root, without braces, quotes, or line breaks.'
}
$appRoot = Join-Path $validationRoot 'PRISMM\ShapeRack'
$installedVst3 = Join-Path $validationRoot 'VST3\ShapeRack.vst3'
$installedClap = Join-Path $validationRoot 'CLAP\ShapeRack.clap'
$presetRoot = Join-Path $validationRoot 'Presets'
$factoryRoot = Join-Path $presetRoot 'Factory'
$factorySource = Join-Path $validationRepoRoot 'presets\Factory'
$editedFactoryPath = $null
$editedFactoryHash = $null
$uninstallerPath = Join-Path $appRoot 'Uninstall\unins000.exe'
$bundleRoot = Join-Path $validationRepoRoot 'target\bundled'

# Refuse to overwrite an earlier staged product or execute an existing uninstaller.
foreach ($ownedPath in @($appRoot, $installedVst3, $installedClap)) {
    if (Test-Path -LiteralPath $ownedPath) {
        throw "A staging payload already exists at $ownedPath. Pass a new, unique -TestRoot."
    }
}
if (Test-Path -LiteralPath $validationRoot) {
    if (-not (Test-Path -LiteralPath $validationRoot -PathType Container)) {
        throw '-TestRoot must be a directory.'
    }
    $oldUninstallers = @(Get-ChildItem -LiteralPath $validationRoot -Filter 'unins*.exe' -File -Recurse)
    if ($oldUninstallers.Count -ne 0) {
        throw 'An existing uninstaller was found under TestRoot. Pass a new, unique directory.'
    }
}

function Assert-FileHash {
    param([string]$Source, [string]$Destination)
    if (-not (Test-Path -LiteralPath $Destination -PathType Leaf)) {
        throw "Installed file is missing: $Destination"
    }
    $expectedHash = (Get-FileHash -LiteralPath $Source -Algorithm SHA256).Hash
    $installedHash = (Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash
    if ($expectedHash -ne $installedHash) {
        throw "Installed file differs from its source: $Destination"
    }
}

function Assert-DirectoryHashes {
    param([string]$Source, [string]$Destination)
    $sourceFiles = @(Get-ChildItem -LiteralPath $Source -File -Recurse)
    if ($sourceFiles.Count -eq 0) { throw "Source payload is empty: $Source" }
    foreach ($sourceFile in $sourceFiles) {
        $relativeFile = $sourceFile.FullName.Substring($Source.TrimEnd('\').Length + 1)
        Assert-FileHash -Source $sourceFile.FullName -Destination (Join-Path $Destination $relativeFile)
    }
    $installedFiles = @(Get-ChildItem -LiteralPath $Destination -File -Recurse)
    if ($installedFiles.Count -ne $sourceFiles.Count) {
        throw "Unexpected files in the installed payload: $Destination"
    }
}

function Invoke-HiddenSetup {
    param([string]$Executable, [string[]]$Arguments, [string]$Operation)
    $process = Start-Process -FilePath $Executable -ArgumentList $Arguments -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        throw "$Operation failed with native exit code $($process.ExitCode). Logs: $validationRoot"
    }
}

function Invoke-BinaryCheck {
    param([string]$Script, [string]$Binary)
    & $Python (Join-Path $PSScriptRoot $Script) $Binary
    if ($LASTEXITCODE -ne 0) {
        throw "$Script failed with native exit code $LASTEXITCODE."
    }
}

$documentationFiles = @('README.md', 'INSTALL.md', 'VALIDATION.md', 'THIRD_PARTY.md', 'LICENSE')
function Assert-InstalledPayload {
    param([bool]$IncludeClap)
    Assert-DirectoryHashes -Source (Join-Path $bundleRoot 'ShapeRack.vst3') -Destination $installedVst3
    if ($IncludeClap) {
        Assert-FileHash -Source (Join-Path $bundleRoot 'ShapeRack.clap') -Destination $installedClap
    } elseif (Test-Path -LiteralPath $installedClap) {
        throw 'The default installation unexpectedly installed the optional CLAP plugin.'
    }
    foreach ($document in $documentationFiles) {
        Assert-FileHash -Source (Join-Path $validationRepoRoot $document) -Destination (Join-Path $appRoot $document)
    }
    Assert-FileHash -Source (Join-Path $validationRepoRoot 'installer\AfterInstall.txt') -Destination (Join-Path $appRoot 'AfterInstall.txt')
    Assert-DirectoryHashes -Source (Join-Path $validationRepoRoot 'licenses') -Destination (Join-Path $appRoot 'licenses')
    Assert-PresetFiles
    if (-not (Test-Path -LiteralPath $uninstallerPath -PathType Leaf)) {
        throw 'The staging installer did not create its expected uninstaller.'
    }
}

function Assert-PresetFiles {
    $factoryFiles = @(Get-ChildItem -LiteralPath $factorySource -Filter '*.shaperack.json' -File)
    if ($factoryFiles.Count -ne 5) { throw 'Expected five bundled factory presets.' }
    foreach ($factoryFile in $factoryFiles) {
        $installedPreset = Join-Path $factoryRoot $factoryFile.Name
        if ($installedPreset -eq $editedFactoryPath) {
            if ((Get-FileHash -LiteralPath $installedPreset -Algorithm SHA256).Hash -ne $editedFactoryHash) {
                throw 'Installation or uninstallation replaced an existing preset.'
            }
        } else {
            Assert-FileHash -Source $factoryFile.FullName -Destination $installedPreset
        }
        $preset = Get-Content -LiteralPath $installedPreset -Raw | ConvertFrom-Json
        if ($preset.format -ne 'shaperack-preset' -or $preset.version -ne 1 -or
            @($preset.parameters.PSObject.Properties).Count -ne 153) {
            throw "Invalid installed factory preset: $installedPreset"
        }
    }
}

function Uninstall-StagedPayload {
    param([string]$Phase)
    Invoke-HiddenSetup -Executable $uninstallerPath -Operation "$Phase uninstall" -Arguments @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART',
        ('/LOG="' + (Join-Path $validationRoot "$Phase-uninstall.log") + '"')
    )
    # Inno's temporary cleanup helper can finish shortly after the uninstaller exits.
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while ((Test-Path -LiteralPath $uninstallerPath) -and [DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 100
    }
    foreach ($ownedPath in @($installedVst3, $installedClap, $uninstallerPath)) {
        if (Test-Path -LiteralPath $ownedPath) {
            throw "Uninstall left an owned payload: $ownedPath"
        }
    }
    foreach ($document in ($documentationFiles + @('AfterInstall.txt'))) {
        if (Test-Path -LiteralPath (Join-Path $appRoot $document)) {
            throw "Uninstall left owned documentation: $document"
        }
    }
    if (Test-Path -LiteralPath (Join-Path $appRoot 'licenses')) {
        $remainingLicenses = @(Get-ChildItem -LiteralPath (Join-Path $appRoot 'licenses') -File -Recurse)
        if ($remainingLicenses.Count -ne 0) { throw 'Uninstall left owned license files.' }
    }
    Assert-PresetFiles
}

# Build precisely the TestRoot variant; this never runs the production installer.
$buildScript = Join-Path $PSScriptRoot 'build-installer.ps1'
$compilerOutput = @(& $buildScript -InnoCompiler $InnoCompiler -TestRoot $validationRoot)
$installerRecords = @($compilerOutput | Where-Object { $_.PSObject.Properties.Name -contains 'FullName' })
if ($installerRecords.Count -ne 1) { throw 'The build script did not return one installer path.' }
$testingInstaller = $installerRecords[0].FullName
if (-not $testingInstaller.EndsWith('-Setup-TEST.exe', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Refusing to execute an installer that is not the staging-only variant.'
}
New-Item -ItemType Directory -Path $validationRoot -Force | Out-Null

Write-Host "Validating isolated staging installer: $testingInstaller"
Write-Host "Staging root: $validationRoot"
$baseArguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', ('/DIR="' + $appRoot + '"'))

# Omit /TYPE to verify the real default component selection.
Invoke-HiddenSetup -Executable $testingInstaller -Operation 'Default install' -Arguments (
    $baseArguments + @('/LOG="' + (Join-Path $validationRoot 'default-install.log') + '"')
)
Assert-InstalledPayload -IncludeClap $false
Invoke-BinaryCheck -Script 'validate_vst3.py' -Binary $installedVst3
Uninstall-StagedPayload -Phase 'default'
Write-Host 'PASS default installation: VST3 only, exact payload hashes, exported factory, clean uninstall.'

# Retained presets must survive a later update, including user edits to a factory copy.
$editedFactoryPath = Join-Path $factoryRoot 'preset-Clean-rack.shaperack.json'
[IO.File]::AppendAllText($editedFactoryPath, "`n ")
$editedFactoryHash = (Get-FileHash -LiteralPath $editedFactoryPath -Algorithm SHA256).Hash
$personalPresetPath = Join-Path $presetRoot 'preset-Personal-test.shaperack.json'
Copy-Item -LiteralPath $editedFactoryPath -Destination $personalPresetPath
$personalPresetHash = (Get-FileHash -LiteralPath $personalPresetPath -Algorithm SHA256).Hash

Invoke-HiddenSetup -Executable $testingInstaller -Operation 'Full install' -Arguments (
    $baseArguments + @('/TYPE=full', ('/LOG="' + (Join-Path $validationRoot 'full-install.log') + '"'))
)
Assert-InstalledPayload -IncludeClap $true
Invoke-BinaryCheck -Script 'validate_vst3.py' -Binary $installedVst3
Invoke-BinaryCheck -Script 'validate_clap.py' -Binary $installedClap

# An unrelated user file must survive uninstall; no recursive deletion is used.
$preservedPath = Join-Path $appRoot ('preserved-user-file-' + [Guid]::NewGuid().ToString('N') + '.txt')
$preservedText = 'Installer validation: this unrelated file must survive uninstall.'
[System.IO.File]::WriteAllText($preservedPath, $preservedText)
Uninstall-StagedPayload -Phase 'full'
if ((Get-FileHash -LiteralPath $personalPresetPath -Algorithm SHA256).Hash -ne $personalPresetHash) {
    throw 'Installation or uninstallation modified a personal preset.'
}
if (-not (Test-Path -LiteralPath $preservedPath -PathType Leaf) -or
    [System.IO.File]::ReadAllText($preservedPath) -ne $preservedText) {
    throw 'Uninstall removed or modified an unrelated user file.'
}
Write-Host 'PASS full installation: VST3 + CLAP, exact payload hashes, real plugin API checks, clean uninstall preserving unrelated files.'
Write-Host 'PASS preset folder: five valid factory files, edited factory copy and personal preset survive updates and uninstall.'
Write-Host "Installer validation passed. Logs and preserved-file fixture remain under: $validationRoot"
