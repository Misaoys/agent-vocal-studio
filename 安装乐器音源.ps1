$ErrorActionPreference = 'Stop'
$sampleDirectory = Join-Path $PSScriptRoot 'runtime\soundfonts'
$sampleFile = Join-Path $sampleDirectory 'GeneralUser-GS.sf2'
$sampleHash = '9575028c7a1f589f5770fccc8cff2734566af40cd26ed836944e9a5152688cfe'
$sampleCommit = '684543d5e5efaef08d02be50dcda8d552478fa60'
$sampleBase = "https://raw.githubusercontent.com/mrbumpy409/GeneralUser-GS/$sampleCommit"
New-Item -ItemType Directory -Force -Path $sampleDirectory | Out-Null
if (-not (Test-Path -LiteralPath $sampleFile)) {
    Invoke-WebRequest -Uri "$sampleBase/GeneralUser-GS.sf2" -OutFile ($sampleFile + '.partial')
    if ((Get-FileHash -LiteralPath ($sampleFile + '.partial') -Algorithm SHA256).Hash.ToLowerInvariant() -ne $sampleHash) { throw 'SoundFont checksum mismatch.' }
    Move-Item -LiteralPath ($sampleFile + '.partial') -Destination $sampleFile
}
if ((Get-FileHash -LiteralPath $sampleFile -Algorithm SHA256).Hash.ToLowerInvariant() -ne $sampleHash) { throw 'SoundFont checksum mismatch.' }
if (-not (Test-Path -LiteralPath (Join-Path $sampleDirectory 'LICENSE.txt'))) { Invoke-WebRequest -Uri "$sampleBase/documentation/LICENSE.txt" -OutFile (Join-Path $sampleDirectory 'LICENSE.txt') }
@{ name='GeneralUser GS'; version='2.0.3'; commit=$sampleCommit; sha256=$sampleHash; source='https://github.com/mrbumpy409/GeneralUser-GS' } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $sampleDirectory 'installed.json') -Encoding utf8
Write-Output 'GeneralUser GS 2.0.3 is ready.'
