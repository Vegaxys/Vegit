# Builds the Windows installer: release build, then Inno Setup.
# Output: target\installer\Vegit-Setup-<version>.exe (the version comes from Cargo.toml).
# Needs Inno Setup 6 (winget install JRSoftware.InnoSetup).

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "Vegit $version"

cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

# build.rs renders the app icon into OUT_DIR; the newest one matches the exe just built.
$ico = Get-ChildItem target\release\build\vegit-*\out\vegit.ico | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $ico) { throw 'vegit.ico not found: build.rs did not run?' }
New-Item -ItemType Directory -Force target\installer | Out-Null
Copy-Item $ico.FullName target\installer\vegit.ico -Force

# Inno Setup installs per user or per machine; it is rarely on PATH.
$iscc = @(
    (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source,
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup 6 not found: winget install JRSoftware.InnoSetup' }

& $iscc /Q "/DAppVersion=$version" "/DIconFile=$root\target\installer\vegit.ico" installer\vegit.iss
if ($LASTEXITCODE -ne 0) { throw 'Inno Setup failed' }
Get-Item "target\installer\Vegit-Setup-$version.exe"
