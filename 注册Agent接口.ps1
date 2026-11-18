$ErrorActionPreference = 'Stop'
$studioHome = [IO.Path]::GetFullPath($PSScriptRoot)
$studioExecutable = Join-Path $studioHome 'bin\agent-vocal-studio.exe'
if (-not (Test-Path -LiteralPath $studioExecutable)) { throw 'Build Agent Vocal Studio first.' }
& codex mcp add agent_vocal_studio -- $studioExecutable --mcp --home $studioHome
if ($LASTEXITCODE -ne 0) { throw 'MCP registration failed.' }
