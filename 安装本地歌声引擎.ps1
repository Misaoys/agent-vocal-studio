$ErrorActionPreference = 'Stop'
$engineRoot = [IO.Path]::GetFullPath($PSScriptRoot)
$engineDownloadDirectory = Join-Path $engineRoot 'runtime\downloads'
$engineRuntimeDirectory = Join-Path $engineRoot 'runtime\voicevox'
$engineArchive = Join-Path $engineDownloadDirectory 'voicevox_engine-windows-cpu-0.26.2.7z.001'
$engineExpectedSha256 = '55824ce0267a121c5d8f47c7d5d82a08b5c072f8417fddd20dc4096ac47aeaaa'
$engineOfficialUrl = 'https://github.com/VOICEVOX/voicevox_engine/releases/download/0.26.2/voicevox_engine-windows-cpu-0.26.2.7z.001'
$engineInstallRecordPath = Join-Path $engineRuntimeDirectory 'installed.json'
if ((Test-Path -LiteralPath $engineInstallRecordPath) -and (Test-Path -LiteralPath (Join-Path $engineRuntimeDirectory 'windows-cpu\run.exe'))) {
    $enginePreviousInstall = Get-Content -LiteralPath $engineInstallRecordPath -Raw | ConvertFrom-Json
    if ($enginePreviousInstall.version -eq '0.26.2' -and $enginePreviousInstall.source_sha256 -eq $engineExpectedSha256) {
        Write-Output 'VOICEVOX 0.26.2 is already installed.'
        exit 0
    }
}
New-Item -ItemType Directory -Force -Path $engineDownloadDirectory,$engineRuntimeDirectory | Out-Null
if (-not (Test-Path -LiteralPath $engineArchive)) {
    Invoke-WebRequest -Uri $engineOfficialUrl -OutFile ($engineArchive + '.partial')
    $engineDigest = (Get-FileHash -LiteralPath ($engineArchive + '.partial') -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($engineDigest -ne $engineExpectedSha256) { throw 'Official VOICEVOX archive SHA-256 mismatch.' }
    Move-Item -LiteralPath ($engineArchive + '.partial') -Destination $engineArchive
}
if ((Get-FileHash -LiteralPath $engineArchive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $engineExpectedSha256) { throw 'Official VOICEVOX archive SHA-256 mismatch.' }
& tar -xf $engineArchive -C $engineRuntimeDirectory
if ($LASTEXITCODE -ne 0) { throw 'Windows tar could not extract the official engine archive.' }
if (-not (Test-Path -LiteralPath (Join-Path $engineRuntimeDirectory 'windows-cpu\run.exe'))) { throw 'Engine executable missing after extraction.' }
@{ version = '0.26.2'; source_sha256 = $engineExpectedSha256 } | ConvertTo-Json | Set-Content -LiteralPath $engineInstallRecordPath -Encoding utf8
Write-Output 'VOICEVOX 0.26.2 is ready. Start Agent Vocal Studio.'
