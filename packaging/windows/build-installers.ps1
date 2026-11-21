# PREVIEW: build the Windows installers from the exe build-exe.ps1 made:
#   dist\Keywhere-<version>-windows-x64-setup.exe  per-user (NSIS), no admin;
#                                                  updates itself in the app
#   dist\Keywhere-<version>-windows-x64.msi         per-machine (WiX), Program
#                                                  Files; updated by the next MSI
# Not code-signed, so SmartScreen warns.
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..\..')
$meta = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
$version = ($meta.packages | Where-Object name -eq 'keywhere').version
$exe = (Resolve-Path target\release\keywhere.exe).Path
$icon = (Resolve-Path packaging\windows\keywhere.ico).Path
$license = (Resolve-Path LICENSE).Path
New-Item -ItemType Directory -Force dist | Out-Null
$dist = (Resolve-Path dist).Path

# NSIS (preinstalled on GitHub's Windows images; else Chocolatey).
$makensis = Join-Path ${env:ProgramFiles(x86)} 'NSIS\makensis.exe'
if (-not (Test-Path $makensis)) {
    choco install nsis --no-progress -y | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'installing NSIS failed' }
}
$setup = Join-Path $dist "Keywhere-$version-windows-x64-setup.exe"
& $makensis /V2 "/DVERSION=$version" "/DEXE=$exe" "/DICON=$icon" "/DLICENSE=$license" "/DOUT=$setup" packaging\windows\keywhere.nsi
if ($LASTEXITCODE -ne 0) { throw 'makensis failed' }

# WiX v5, pinned.
if (-not (Get-Command wix -ErrorAction SilentlyContinue)) {
    dotnet tool install --global wix --version 5.0.2 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'installing WiX failed' }
    $env:PATH += ";$env:USERPROFILE\.dotnet\tools"
}
$msi = Join-Path $dist "Keywhere-$version-windows-x64.msi"
wix build -arch x64 -d "Version=$version" -d "Exe=$exe" -d "Icon=$icon" -d "License=$license" -o $msi packaging\windows\keywhere.wxs
if ($LASTEXITCODE -ne 0) { throw 'wix build failed' }
Remove-Item (Join-Path $dist '*.wixpdb') -ErrorAction SilentlyContinue

Write-Output $setup
Write-Output $msi
