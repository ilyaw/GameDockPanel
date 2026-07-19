#Requires -Version 5.1
<#
.SYNOPSIS
  Clone GameDockPanel (if needed), install npm deps, and build on Windows.

.DESCRIPTION
  Checks Git, Node.js, Rust, and MSVC linker, then:
    git clone  ->  npm install  ->  npm run tauri build

  Prerequisites (install manually if missing):
    - Git: https://git-scm.com/download/win
    - Node.js LTS: https://nodejs.org
    - Rust: https://rustup.rs
    - VS Build Tools (C++): https://visualstudio.microsoft.com/visual-cpp-build-tools/
    - WebView2 Runtime (usually preinstalled on Win10/11)

.PARAMETER CloneDir
  Where to clone the repository (default: Desktop\GameDockPanel).

.PARAMETER RepoUrl
  Git remote URL.

.PARAMETER Dev
  Run `npm run tauri dev` instead of a release build.

.PARAMETER SkipClone
  Skip git clone/pull — use when the repo is already on disk.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\windows-setup-and-build.ps1

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File .\windows-setup-and-build.ps1 -Dev
#>

[CmdletBinding()]
param(
    [string] $CloneDir = (Join-Path $env:USERPROFILE "Desktop\GameDockPanel"),
    [string] $RepoUrl = "https://github.com/ilyaw/GameDockPanel.git",
    [switch] $Dev,
    [switch] $SkipClone
)

$ErrorActionPreference = "Stop"

function Write-Step([string] $Message) {
    Write-Host ""
    Write-Host "==> $Message" -ForegroundColor Cyan
}

function Write-Ok([string] $Message) {
    Write-Host "    OK  $Message" -ForegroundColor Green
}

function Write-Warn([string] $Message) {
    Write-Host "    !!  $Message" -ForegroundColor Yellow
}

function Write-Fail([string] $Message) {
    Write-Host "    XX  $Message" -ForegroundColor Red
}

function Test-Command([string] $Name) {
    return [bool](Get-Command $Name -ErrorAction SilentlyContinue)
}

function Get-VersionLine([string] $Name) {
    try {
        $raw = & $Name --version 2>&1 | Select-Object -First 1
        return "$Name $raw"
    } catch {
        return "$Name (version unknown)"
    }
}

function Test-MsvcLinker {
    if (Test-Command "where.exe") {
        $link = & where.exe link.exe 2>$null | Select-Object -First 1
        if ($link) { return $true }
    }
    $vsPaths = @(
        "${env:ProgramFiles(x86)}\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC",
        "${env:ProgramFiles}\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC",
        "${env:ProgramFiles}\Microsoft Visual Studio\2022\Professional\VC\Tools\MSVC",
        "${env:ProgramFiles}\Microsoft Visual Studio\2022\Enterprise\VC\Tools\MSVC"
    )
    foreach ($base in $vsPaths) {
        if (Test-Path $base) { return $true }
    }
    return $false
}

function Ensure-Prerequisites {
    Write-Step "Checking prerequisites"

    $missing = @()

    if (-not (Test-Command "git")) { $missing += "Git — https://git-scm.com/download/win" }
    else { Write-Ok (Get-VersionLine "git") }

    if (-not (Test-Command "node")) { $missing += "Node.js LTS — https://nodejs.org" }
    else { Write-Ok (Get-VersionLine "node") }

    if (-not (Test-Command "npm")) { $missing += "npm (comes with Node.js)" }
    else { Write-Ok (Get-VersionLine "npm") }

    if (-not (Test-Command "cargo")) {
        $missing += "Rust (rustup) — https://rustup.rs — then restart the terminal"
    } else {
        Write-Ok (Get-VersionLine "rustc")
        Write-Ok (Get-VersionLine "cargo")
    }

    if (-not (Test-MsvcLinker)) {
        $missing += @(
            "MSVC C++ build tools (link.exe)",
            "Install: https://visualstudio.microsoft.com/visual-cpp-build-tools/",
            "Workload: Desktop development with C++"
        )
    } else {
        Write-Ok "MSVC linker (link.exe) found"
    }

    if ($missing.Count -gt 0) {
        Write-Fail "Missing prerequisites:"
        foreach ($item in $missing) { Write-Host "      - $item" }
        Write-Host ""
        Write-Host "Install the items above, reopen PowerShell, and run this script again." -ForegroundColor Yellow
        exit 1
    }
}

function Sync-Repository {
    param([string] $Root, [string] $Url)

    Write-Step "Repository: $Root"

    if (Test-Path (Join-Path $Root ".git")) {
        Write-Ok "Repo exists — pulling latest main"
        Push-Location $Root
        try {
            & git fetch origin main 2>&1 | Write-Host
            if ($LASTEXITCODE -ne 0) { throw "git fetch failed (exit $LASTEXITCODE)" }

            & git checkout main 2>&1 | Write-Host
            if ($LASTEXITCODE -ne 0) { throw "git checkout main failed (exit $LASTEXITCODE)" }

            # Build machine must track origin/main. Local edits (often Cargo.toml
            # from a half-applied patch) block `pull --ff-only` — stash them.
            $porcelain = & git status --porcelain
            if ($porcelain) {
                Write-Warn "Local changes would block pull — stashing:"
                $porcelain | ForEach-Object { Write-Host "      $_" -ForegroundColor Yellow }
                & git stash push -u -m "windows-setup-and-build auto-stash" 2>&1 | Write-Host
                if ($LASTEXITCODE -ne 0) {
                    Write-Warn "stash failed — discarding tracked local edits (build machine)"
                    & git checkout -- . 2>&1 | Write-Host
                    & git clean -fd --exclude=node_modules --exclude=src-tauri/target 2>&1 | Write-Host
                }
            }

            & git pull --ff-only origin main 2>&1 | Write-Host
            if ($LASTEXITCODE -ne 0) {
                Write-Fail "git pull не удался. Разреши конфликты вручную или удали папку и запусти снова."
                throw "git pull --ff-only failed (exit $LASTEXITCODE)"
            }
            Write-Ok "On latest origin/main"
        } finally {
            Pop-Location
        }
        return
    }

    if (Test-Path $Root) {
        Write-Fail "Path exists but is not a git repo: $Root"
        Write-Host "      Remove or rename the folder, or pass -CloneDir to another path." -ForegroundColor Yellow
        exit 1
    }

    $parent = Split-Path $Root -Parent
    if (-not (Test-Path $parent)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }

    Write-Ok "Cloning $Url"
    & git clone --branch main --depth 1 $Url $Root
    if ($LASTEXITCODE -ne 0) { throw "git clone failed (exit $LASTEXITCODE)" }
}

function Invoke-AppBuild {
    param([string] $AppDir, [bool] $RunDev)

    Write-Step "npm install"
    Push-Location $AppDir
    try {
        & npm install
        if ($LASTEXITCODE -ne 0) { throw "npm install failed (exit $LASTEXITCODE)" }

        if ($RunDev) {
            Write-Step "Starting dev build (npm run tauri dev)"
            Write-Warn "Close the app window or press Ctrl+C in this terminal to stop."
            & npm run tauri dev
        } else {
            Write-Step "Release build (npm run tauri build) — first run can take 10–20 min"
            & npm run tauri build
            if ($LASTEXITCODE -ne 0) { throw "tauri build failed (exit $LASTEXITCODE)" }

            $bundleDir = Join-Path $AppDir "src-tauri\target\release\bundle"
            Write-Step "Build complete"
            if (Test-Path $bundleDir) {
                Write-Ok "Installers / binaries:"
                Get-ChildItem -Path $bundleDir -Recurse -Include *.exe,*.msi,*.nsis.zip -ErrorAction SilentlyContinue |
                    ForEach-Object { Write-Host "      $($_.FullName)" -ForegroundColor Green }
                Write-Host ""
                Write-Host "Opening bundle folder..." -ForegroundColor Cyan
                Start-Process explorer.exe $bundleDir
            } else {
                Write-Warn "Bundle folder not found at $bundleDir"
                Write-Ok "Binary may be at: $(Join-Path $AppDir 'src-tauri\target\release\gamedockpanel.exe')"
            }
        }
    } finally {
        Pop-Location
    }
}

# --- main ---

Write-Host ""
Write-Host " GameDockPanel — Windows setup & build" -ForegroundColor Magenta
Write-Host " =====================================" -ForegroundColor Magenta

Ensure-Prerequisites

if (-not $SkipClone) {
    Sync-Repository -Root $CloneDir -Url $RepoUrl
} else {
    Write-Step "Skipping clone (-SkipClone)"
    if (-not (Test-Path $CloneDir)) {
        Write-Fail "CloneDir does not exist: $CloneDir"
        exit 1
    }
}

$AppDir = Join-Path $CloneDir "GameDockPanel"
if (-not (Test-Path (Join-Path $AppDir "package.json"))) {
    Write-Fail "App folder not found: $AppDir"
    Write-Host "      Expected package.json inside GameDockPanel\GameDockPanel\" -ForegroundColor Yellow
    exit 1
}

Invoke-AppBuild -AppDir $AppDir -RunDev ([bool]$Dev)

Write-Host ""
Write-Host "Done." -ForegroundColor Green
