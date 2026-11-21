# PREVIEW: build dist\Keywhere-<version>-windows-x64-preview.zip from a
# release build. Not code-signed, so SmartScreen warns on first start.
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..\..')
$meta = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
$version = ($meta.packages | Where-Object name -eq 'keywhere').version
cargo build --locked --release -p keywhere
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

# The exe must not need the Visual C++ Redistributable (see .cargo/config.toml).
$bytes = [IO.File]::ReadAllText((Resolve-Path target\release\keywhere.exe), [Text.Encoding]::Latin1)
if ($bytes -match 'VCRUNTIME140') { throw 'keywhere.exe imports VCRUNTIME140.dll' }

$work = Join-Path ([IO.Path]::GetTempPath()) "keywhere-zip-$PID"
New-Item -ItemType Directory -Force $work | Out-Null
Copy-Item target\release\keywhere.exe $work
Copy-Item LICENSE $work
@"
Keywhere $version for Windows - PREVIEW

This build is not code-signed yet: Windows SmartScreen may say it
"protected your PC". Choose "More info" > "Run anyway" only if the zip's
SHA-256 matches SHA256SUMS on the release page.

Keywhere locks the vault when you lock Windows (Win+L), switch user, the
display turns off, or the PC sleeps. Not yet on Windows: a tray icon,
opening a second vault in the running app, and in-app updates (download
new versions from the release page).
"@ | Set-Content -Encoding utf8 (Join-Path $work 'README.txt')

New-Item -ItemType Directory -Force dist | Out-Null
$out = "dist\Keywhere-$version-windows-x64-preview.zip"
if (Test-Path $out) { Remove-Item $out }
Compress-Archive -Path (Join-Path $work '*') -DestinationPath $out
Remove-Item -Recurse $work
Write-Output $out
