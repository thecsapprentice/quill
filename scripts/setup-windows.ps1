<#
.SYNOPSIS
    Installs the tools to build and test Quill on Windows, using Chocolatey.

.DESCRIPTION
    Installs, if missing:
      - The MSVC C++ tools (compiler, linker and Windows SDK). Rust on
        Windows links with MSVC, and the TLS crypto library (aws-lc-rs)
        compiles C code. Any Visual Studio edition that has them already
        (Community, Professional, Enterprise or Build Tools) is used as is;
        only if there is no Visual Studio at all are the 2022 Build Tools
        installed.
      - NASM, the assembler aws-lc-rs uses on 64-bit Windows.
      - Git.
      - rustup, then the Rust version this project pins in
        rust-toolchain.toml (with rustfmt and clippy). rustup's self-update
        is turned off, since Chocolatey manages rustup.

    Safe to run again: anything already installed is left as it is.
    Needs an elevated (Administrator) PowerShell, as Chocolatey does.
    The Build Tools download is a few GB and takes a while.

.PARAMETER Build
    After installing, build the `quill` CLI in release mode and print where
    it is. Run from the repository's root folder.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\setup-windows.ps1 -Build
#>
#Requires -RunAsAdministrator
[CmdletBinding()]
param(
    [switch]$Build
)

$ErrorActionPreference = 'Stop'

# Keep in step with rust-toolchain.toml.
$RustVersion = '1.97.0'

function Write-Step($Text) {
    Write-Host ''
    Write-Host "==> $Text" -ForegroundColor Cyan
}

function Test-Command($Name) {
    return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

function Update-Path {
    # Pick up PATH changes made by installers, without opening a new window.
    $machine = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $user = [Environment]::GetEnvironmentVariable('Path', 'User')
    $env:Path = "$machine;$user"
    $cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
    if ((Test-Path $cargoBin) -and ($env:Path -notlike "*$cargoBin*")) {
        $env:Path = "$env:Path;$cargoBin"
    }
}

if (-not (Test-Command 'choco')) {
    throw 'Chocolatey (choco) was not found. Install it from https://chocolatey.org/install and run this again.'
}

# Chocolatey 2 lists installed packages by default and dropped --local-only.
$chocoMajor = [int]((choco --version) -split '\.')[0]
$listLocal = if ($chocoMajor -ge 2) { @() } else { @('--local-only') }

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'

# Any Visual Studio edition with the x64 C++ compiler and linker.
function Find-Msvc {
    if (-not (Test-Path $vswhere)) { return $null }
    return & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
}

# Any Visual Studio edition at all, with or without the C++ tools.
function Find-VisualStudio {
    if (-not (Test-Path $vswhere)) { return $null }
    return & $vswhere -latest -products * -property displayName
}

function Install-ChocoPackage($Package, [string]$Params = '') {
    $installed = choco list @listLocal --exact $Package --limit-output 2>$null
    if ($installed) {
        Write-Host "$Package is already installed."
        return
    }
    if ($Params) {
        choco install $Package -y --no-progress --package-parameters="$Params"
    } else {
        choco install $Package -y --no-progress
    }
    # 3010 means "installed, restart needed", which Build Tools often says.
    if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne 3010) {
        throw "Installing $Package failed (exit code $LASTEXITCODE)."
    }
}

Write-Step 'MSVC C++ tools (compiler, linker and Windows SDK)'
$msvc = Find-Msvc
$vs = Find-VisualStudio
if ($msvc) {
    Write-Host "Found the C++ tools in $msvc"
} elseif ($vs) {
    # Don't install a second Visual Studio next to the user's own.
    throw @"
$vs is installed but without the C++ tools Rust needs. Open the Visual Studio
Installer, choose Modify on $vs, tick the "Desktop development with C++"
workload (it includes the MSVC tools and the Windows SDK), install, then run
this script again.
"@
} else {
    Install-ChocoPackage 'visualstudio2022buildtools' '--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
}

Write-Step 'NASM (assembler for the TLS crypto library)'
Install-ChocoPackage 'nasm'
# The NASM installer doesn't always add itself to PATH.
$nasmDir = Join-Path $env:ProgramFiles 'NASM'
if (Test-Path (Join-Path $nasmDir 'nasm.exe')) {
    $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    if ($machinePath -notlike "*$nasmDir*") {
        [Environment]::SetEnvironmentVariable('Path', "$machinePath;$nasmDir", 'Machine')
        Write-Host "Added $nasmDir to the system PATH."
    }
}

Write-Step 'Git'
if (Test-Command 'git') {
    Write-Host 'git is already installed.'
} else {
    Install-ChocoPackage 'git'
}

Write-Step 'rustup'
Update-Path
if (Test-Command 'rustup') {
    Write-Host 'rustup is already installed.'
} else {
    Install-ChocoPackage 'rustup.install'
    Update-Path
}
if (-not (Test-Command 'rustup')) {
    throw 'rustup was installed but is not on PATH yet. Open a new Administrator PowerShell and run this script again.'
}

Write-Step "Rust $RustVersion with rustfmt and clippy"
# Chocolatey manages rustup itself; rustup replacing its own files would
# fail ("Access is denied") and isn't wanted. Upgrade with `choco upgrade
# rustup.install` instead.
rustup set auto-self-update disable
if ($LASTEXITCODE -ne 0) { throw 'rustup could not turn off self-update.' }
rustup toolchain install $RustVersion --profile minimal --component rustfmt --component clippy
if ($LASTEXITCODE -ne 0) { throw 'rustup could not install the toolchain.' }
# Outside this repository (where rust-toolchain.toml decides), use the same
# version unless a default is already set.
$null = rustup default 2>$null
if ($LASTEXITCODE -ne 0) {
    rustup default $RustVersion
    if ($LASTEXITCODE -ne 0) { throw 'rustup could not set the default toolchain.' }
} else {
    Write-Host "Keeping your default toolchain: $(rustup default)"
}

Write-Step 'Checking the tools'
Update-Path
$ok = $true
foreach ($tool in @(
        @{ Name = 'rustc'; Args = @('--version') },
        @{ Name = 'cargo'; Args = @('--version') },
        @{ Name = 'nasm'; Args = @('-v') },
        @{ Name = 'git'; Args = @('--version') }
    )) {
    if (Test-Command $tool.Name) {
        $version = & $tool.Name @($tool.Args) 2>&1 | Select-Object -First 1
        Write-Host ("{0,-6} {1}" -f $tool.Name, $version)
    } else {
        Write-Host ("{0,-6} NOT FOUND" -f $tool.Name) -ForegroundColor Red
        $ok = $false
    }
}
$msvc = Find-Msvc
if ($msvc) {
    Write-Host "MSVC   $msvc"
} else {
    Write-Host 'MSVC   NOT FOUND (the C++ build tools are missing)' -ForegroundColor Red
    $ok = $false
}
if (-not $ok) {
    throw 'Some tools are missing; see above. A restart may be needed after the Build Tools install.'
}

if ($Build) {
    Write-Step 'Building the quill CLI (release)'
    if (-not (Test-Path 'Cargo.toml') -or -not (Test-Path 'rust-toolchain.toml')) {
        throw 'Run this from the root of the quill repository to build.'
    }
    cargo build --release --locked -p quill
    if ($LASTEXITCODE -ne 0) { throw 'The build failed; see the errors above.' }
    $exe = Resolve-Path 'target\release\quill.exe'
    Write-Host ''
    Write-Host "Built: $exe" -ForegroundColor Green
}

Write-Host ''
Write-Host 'Done. Open a new PowerShell window so every tool is on PATH.' -ForegroundColor Green
