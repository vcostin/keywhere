# PREVIEW: build dist\Keywhere-<version>-windows-x64-portable.exe, a single
# exe that runs without installing (static C runtime, icon and version info
# embedded; settings go to %APPDATA%\keywhere). Not code-signed, so
# SmartScreen warns on first start.
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..\..')
$meta = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
$version = ($meta.packages | Where-Object name -eq 'keywhere').version
cargo build --locked --release -p keywhere
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$exe = Resolve-Path target\release\keywhere.exe
# Standalone means no Visual C++ Redistributable (see .cargo/config.toml).
$bytes = [IO.File]::ReadAllText($exe, [Text.Encoding]::Latin1)
if ($bytes -match 'VCRUNTIME140') { throw 'keywhere.exe imports VCRUNTIME140.dll' }
$info = (Get-Item $exe).VersionInfo
if ($info.ProductVersion -ne $version) { throw "version resource is '$($info.ProductVersion)', expected $version" }

New-Item -ItemType Directory -Force dist | Out-Null
$out = "dist\Keywhere-$version-windows-x64-portable.exe"
Copy-Item $exe $out -Force
Write-Output $out
