param([switch]$IncludeUI)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$env:CARGO_HOME = Join-Path $projectRoot '.build\cargo'
$env:CARGO_TARGET_DIR = Join-Path $projectRoot '.build\target'
$env:TEMP = Join-Path $projectRoot '.build'
$env:TMP = $env:TEMP
$env:FAST_ASR_HOME = $projectRoot
$env:PYTHONDONTWRITEBYTECODE = '1'
$exePath = Join-Path $env:CARGO_TARGET_DIR 'release\fast-asr.exe'
$exeHash = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash.ToLowerInvariant()
$reportRoot = Join-Path $projectRoot 'reports'
New-Item -ItemType Directory -Force -Path $reportRoot | Out-Null
if (-not ('FastAsrNativeWait' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class FastAsrNativeWait {
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);
}
'@
}
function Invoke-Diagnostic([string[]]$Arguments, [string]$ReportName) {
    $begin = [DateTime]::UtcNow
    $windowStyle = if ($ReportName -in @('input-probe.json', 'ui-probe.json', 'hotkey-probe.json')) { 'Normal' } else { 'Hidden' }
    $process = Start-Process -FilePath $exePath -ArgumentList $Arguments -WorkingDirectory $projectRoot -WindowStyle $windowStyle -PassThru
    $timeoutMs = if ($windowStyle -eq 'Normal') { 15000 } else { 60000 }
    $waitResult = [FastAsrNativeWait]::WaitForSingleObject($process.Handle, $timeoutMs)
    if ($waitResult -ne 0) {
        throw "Diagnostic did not finish: $ReportName (PID $($process.Id)). No further diagnostics will start. Inspect this process before retrying; a termination request does not confirm exit."
    }
    $process.WaitForExit()
    if ($process.ExitCode -ne 0) { throw "Diagnostic failed: $ReportName" }
    $reportPath = Join-Path $reportRoot $ReportName
    $report = Get-Content -LiteralPath $reportPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if (-not $report.passed -or $report.executable_sha256 -ne $exeHash -or (Get-Item -LiteralPath $reportPath).LastWriteTimeUtc -lt $begin.AddSeconds(-1)) {
        throw "Failed, stale, or mismatched report: $reportName"
    }
    Write-Output "$ReportName passed"
}
# A process with an assigned exit code can still be waiting for Windows
# termination cleanup. Enumerate actual process entries instead of HasExited.
if ($IncludeUI) {
    $workspacePrefix = $projectRoot.TrimEnd('\') + '\'
    $existingDiagnostics = @(Get-CimInstance Win32_Process -Filter "Name='fast-asr.exe' OR Name='FastASR.exe'" -ErrorAction Stop | Where-Object {
        $_.ExecutablePath -and $_.ExecutablePath.StartsWith($workspacePrefix, [StringComparison]::OrdinalIgnoreCase) -and
        $_.CommandLine -match '--(?:hotkey|input|ui)-probe(?:\s|$)'
    })
    if ($existingDiagnostics.Count -gt 0) {
        $diagnosticIds = ($existingDiagnostics | ForEach-Object { $_.ProcessId }) -join ', '
        throw "Desktop verification blocked by existing diagnostic processes: $diagnosticIds. No new test windows or input hooks were started. Investigate incomplete exit before retrying."
    }
}
Push-Location -LiteralPath $projectRoot
try {
    python scripts/check_licenses.py
    if ($LASTEXITCODE -ne 0) { throw 'Upstream notice integrity check failed' }
    python scripts/test_licenses.py
    if ($LASTEXITCODE -ne 0) { throw 'Upstream notice regression failed' }
    cargo fmt --check
    if ($LASTEXITCODE -ne 0) { throw 'Formatting failed' }
    cargo test --locked
    if ($LASTEXITCODE -ne 0) { throw 'Tests failed' }
    cargo clippy --locked --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Clippy failed' }
    python scripts/check_hook_lifecycle.py
    if ($LASTEXITCODE -ne 0) { throw 'Private desktop hook cleanup check failed; no further diagnostics started' }
    if ($IncludeUI) { Invoke-Diagnostic @('--hotkey-probe') 'hotkey-probe.json' }
    Invoke-Diagnostic @('--pipeline-probe') 'pipeline-probe.json'
    Invoke-Diagnostic @('--self-test','--model','x-asr-zh-en-480ms','--report','reports/xasr-self-test.json') 'xasr-self-test.json'
    if ($IncludeUI) {
        Invoke-Diagnostic @('--input-probe') 'input-probe.json'
        Invoke-Diagnostic @('--ui-probe') 'ui-probe.json'
    }
} finally { Pop-Location }
