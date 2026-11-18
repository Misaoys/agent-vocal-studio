$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot
$localCargoExecutable = Join-Path $PSScriptRoot 'runtime\toolchain\cargo\bin\cargo.exe'
if (Test-Path -LiteralPath $localCargoExecutable) {
    $env:CARGO_HOME = Join-Path $PSScriptRoot 'runtime\toolchain\cargo'
    $env:RUSTUP_HOME = Join-Path $PSScriptRoot 'runtime\toolchain\rustup'
    $env:PATH = "$env:CARGO_HOME\bin;$env:PATH"
} elseif (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'Install Rust with the MSVC toolchain before building this source package.'
}
& cargo build --release --locked
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
New-Item -ItemType Directory -Force -Path 'bin' | Out-Null
Copy-Item -LiteralPath 'target\release\agent-vocal-studio.exe' -Destination 'bin\agent-vocal-studio.exe'
Copy-Item -LiteralPath 'target\release\agent-vocal-desktop.exe' -Destination 'bin\agent-vocal-desktop.exe'
