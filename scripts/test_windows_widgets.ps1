#requires -Version 7.4
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$ProviderPath,
    # Legacy switch name retained for compatibility; this is an activation test.
    [switch]$PackagedProxy,
    [string]$LogDirectory = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$provider = (Resolve-Path -LiteralPath $ProviderPath -ErrorAction Stop).Path
if (-not (Test-Path -LiteralPath $provider -PathType Leaf)) { throw 'ProviderPath must be a file.' }

# Self-test mode does not read user configuration or contact the player/Widgets.
# Logs live outside the package staging tree, which a failed build removes.
if (-not $LogDirectory) {
    $LogDirectory = Join-Path ([IO.Path]::GetTempPath()) ('wocli-widgets-self-test-' + [Guid]::NewGuid().ToString('N'))
}
New-Item -ItemType Directory -Path $logDirectory -Force | Out-Null
$stdout = Join-Path $logDirectory 'stdout.log'
$stderr = Join-Path $logDirectory 'stderr.log'
$process = $null
try {
    $argument = if ($PackagedProxy) { '--self-test-packaged-proxy' } else { '--self-test' }
    $scope = if ($PackagedProxy) { 'package identity and LOCAL_SERVER COM activation of a same-process temporary class' } else { 'in-process COM factory and interfaces' }
    Write-Host "Provider self-test scope: $scope. Cross-process proxy marshaling and Widgets host/display integration are not verified."
    $process = Start-Process -FilePath $provider -ArgumentList $argument -PassThru `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    if (-not $process.WaitForExit(60000)) {
        $process.Kill($true)
        # Bound the drain after termination so redirected streams can flush
        # without introducing an unbounded wait after the 60-second deadline.
        [void]$process.WaitForExit(5000)
        throw 'Provider COM self-test timed out after 60 seconds.'
    }
    if ($process.ExitCode -ne 0) { throw "Provider COM self-test failed ($($process.ExitCode))." }
    Write-Host 'Provider COM self-test passed.'
} finally {
    if ($process) { $process.Dispose() }
    foreach ($log in @($stdout, $stderr)) {
        Write-Host "Provider COM self-test log: $log"
        try {
            if (Test-Path -LiteralPath $log -PathType Leaf) {
                $content = Get-Content -LiteralPath $log -Raw
                if ($content) { Write-Host $content } else { Write-Host '(empty)' }
            } else {
                Write-Host '(not created)'
            }
        } catch {
            # A logging failure must not hide the original process failure or
            # prevent the other stream from being printed.
            Write-Warning "Could not read self-test log $log : $_"
        }
    }
}
