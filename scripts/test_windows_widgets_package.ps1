#requires -Version 5.1
# CI-only integration test. The ordinary build script never installs packages or
# changes Developer Mode. Appx cmdlets run in Windows PowerShell; the package
# context launches PowerShell 7 for the shared, bounded provider test helper.
[CmdletBinding()]
param([Parameter(Mandatory)][string]$PreviewDirectory)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Package registration tests are restricted to GitHub Actions runners.' }
if ($env:OS -ne 'Windows_NT' -or $PSVersionTable.PSEdition -ne 'Desktop') {
    throw 'Run this CI helper in Windows PowerShell 5.1 (workflow shell: powershell).'
}
if (-not $env:RUNNER_TEMP) { throw 'RUNNER_TEMP is required for isolated CI test logs.' }
$runnerTemp = (Resolve-Path -LiteralPath $env:RUNNER_TEMP).Path
$localAppData = [Environment]::GetFolderPath('LocalApplicationData').TrimEnd('\') + '\'
if (($runnerTemp.TrimEnd('\') + '\').StartsWith($localAppData, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'RUNNER_TEMP must be outside LocalAppData to avoid package file virtualization of the result.'
}
$preview = (Resolve-Path -LiteralPath $PreviewDirectory).Path
$layout = Join-Path $preview 'Layout'
$manifestPath = Join-Path $layout 'AppxManifest.xml'
$buildInfoPath = Join-Path $preview 'build-info.json'
[xml]$manifest = Get-Content -LiteralPath $manifestPath -Raw
$buildInfo = Get-Content -LiteralPath $buildInfoPath -Raw | ConvertFrom-Json
$packageName = [string]$manifest.Package.Identity.Name
if ($packageName -ne 'Wocli.Music.WidgetsPreview' -or
    [string]$manifest.Package.Identity.Publisher -ne 'CN=wocli-music.WidgetsPreview') {
    throw 'Refusing to register a package other than the wocli Widgets preview.'
}
$nativeArchitecture = $env:PROCESSOR_ARCHITECTURE.ToLowerInvariant().Replace('amd64', 'x64')
if ($buildInfo.architecture -ne $nativeArchitecture) { throw 'Package self-test requires its native runner architecture.' }
$msix = @(Get-ChildItem -LiteralPath $preview -File -Filter 'wocli-system-widgets-preview-*.msix')
if ($msix.Count -ne 1 -or (Get-FileHash -LiteralPath $msix[0].FullName -Algorithm SHA256).Hash -ne $buildInfo.sha256) {
    throw 'The MakeAppx-validated MSIX is missing or does not match build-info.json.'
}
$buildInfo | Add-Member -NotePropertyName providerPackagedActivationSelfTestPassed -NotePropertyValue $false -Force
function Save-BuildInfo {
    [IO.File]::WriteAllText($buildInfoPath, ($buildInfo | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))
}
Save-BuildInfo

Import-Module Appx -ErrorAction Stop
foreach ($command in @('Add-AppxPackage', 'Remove-AppxPackage', 'Get-AppxPackage', 'Invoke-CommandInDesktopPackage')) {
    Get-Command $command -ErrorAction Stop | Out-Null
}
if (@(Get-AppxPackage -Name $packageName).Count) { throw 'A preview is already registered; refusing to replace it during a CI test.' }
$pwsh = (Get-Command pwsh.exe -CommandType Application -ErrorAction Stop).Source
$helper = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'test_windows_widgets.ps1')).Path
$runToken = [Guid]::NewGuid().ToString('N')
$testDirectory = Join-Path $runnerTemp ('wocli-widgets-package-test-' + $runToken)
New-Item -ItemType Directory -Path $testDirectory | Out-Null
$resultPath = Join-Path $testDirectory 'result.json'
$startedPath = Join-Path $testDirectory 'started.json'
$wrapperLog = Join-Path $testDirectory 'wrapper.log'
$providerLogDirectory = Join-Path $testDirectory 'provider'
$configPath = Join-Path $testDirectory 'test-config.json'
@{
    token = $runToken; helper = $helper; provider = (Join-Path $layout 'Wocli.Widgets.exe')
    result = $resultPath; started = $startedPath; log = $wrapperLog; providerLogs = $providerLogDirectory
} | ConvertTo-Json | Set-Content -LiteralPath $configPath -Encoding UTF8

# JSON transports paths without interpolating them into executable PowerShell.
# The outer token checks and atomic rename prevent a partial/stale result from
# being interpreted as success. No config/daemon/live Widgets APIs are used.
$wrapper = @'
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$config = Get-Content -LiteralPath $CONFIG_PATH -Raw | ConvertFrom-Json
$identity = [Diagnostics.Process]::GetCurrentProcess()
@{ token = $config.token; pid = $PID; startTicks = $identity.StartTime.ToUniversalTime().Ticks } |
    ConvertTo-Json | Set-Content -LiteralPath ($config.started + '.tmp') -Encoding utf8
Move-Item -LiteralPath ($config.started + '.tmp') -Destination $config.started
$exitCode = 1
try {
    & $config.helper -ProviderPath $config.provider -PackagedProxy -LogDirectory $config.providerLogs *>> $config.log
    $exitCode = 0
} catch {
    ($_ | Out-String) | Add-Content -LiteralPath $config.log -Encoding utf8
} finally {
    @{ token = $config.token; exitCode = $exitCode } | ConvertTo-Json |
        Set-Content -LiteralPath ($config.result + '.tmp') -Encoding utf8
    Move-Item -LiteralPath ($config.result + '.tmp') -Destination $config.result
}
exit $exitCode
'@
$wrapper = $wrapper.Replace('$CONFIG_PATH', "'" + $configPath.Replace("'", "''") + "'")
$encodedWrapper = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($wrapper))

function Stop-TestProcess {
    if (-not (Test-Path -LiteralPath $startedPath -PathType Leaf)) { return }
    $started = Get-Content -LiteralPath $startedPath -Raw | ConvertFrom-Json
    if ($started.token -ne $runToken) { throw 'Unexpected test process token.' }
    $running = Get-Process -Id $started.pid -ErrorAction SilentlyContinue
    if ($running -and $running.StartTime.ToUniversalTime().Ticks -eq $started.startTicks) {
        # Windows PowerShell/.NET Framework lacks Process.Kill(entireProcessTree).
        & "$env:SystemRoot\System32\taskkill.exe" /PID $started.pid /T /F | Out-Host
        if ($LASTEXITCODE -ne 0 -and (Get-Process -Id $started.pid -ErrorAction SilentlyContinue)) {
            throw 'Could not terminate the package test process tree.'
        }
    }
}

$registryPath = 'SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock'
$registry = $null
$registryExisted = $false
$registryBackup = @{}
$registrationAttempted = $false
$testPassed = $false
$cleanupErrors = [Collections.Generic.List[string]]::new()
try {
    $registry = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey($registryPath, $true)
    $registryExisted = $null -ne $registry
    if (-not $registry) { $registry = [Microsoft.Win32.Registry]::LocalMachine.CreateSubKey($registryPath) }
    foreach ($name in @('AllowDevelopmentWithoutDevLicense', 'AllowAllTrustedApps')) {
        $exists = $registry.GetValueNames() -contains $name
        $registryBackup[$name] = @{ exists = $exists }
        if ($exists) {
            $registryBackup[$name].value = $registry.GetValue($name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            $registryBackup[$name].kind = $registry.GetValueKind($name)
        }
        $registry.SetValue($name, 1, [Microsoft.Win32.RegistryValueKind]::DWord)
    }

    $frameworks = @(Get-ChildItem -LiteralPath (Join-Path $preview 'Dependencies') -File -Filter '*.msix')
    if ($frameworks.Count -ne 1) { throw 'Expected exactly one bundled Microsoft runtime framework.' }
    $framework = $frameworks[0]
    $signature = Get-AuthenticodeSignature -LiteralPath $framework.FullName
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation(?:,|$)' -or
        (Get-FileHash -LiteralPath $framework.FullName -Algorithm SHA256).Hash -ne $buildInfo.runtimeFramework.sha256) {
        throw 'Bundled runtime framework signature/hash verification failed.'
    }
    # This Microsoft framework is shared. Leave it installed on the temporary
    # runner; only our preview registration and Developer Mode changes are undone.
    Add-AppxPackage -Path $framework.FullName
    $registrationAttempted = $true
    Add-AppxPackage -Register $manifestPath
    $packages = @(Get-AppxPackage -Name $packageName | Where-Object { $_.InstallLocation -eq $layout })
    if ($packages.Count -ne 1) { throw 'The expected development Layout was not registered.' }
    $package = $packages[0]
    Write-Host "Testing package identity and LOCAL_SERVER COM activation of a same-process temporary class: $($package.PackageFullName)"
    Write-Host 'This does not verify cross-process proxy marshaling or Widgets host/display integration.'
    # https://learn.microsoft.com/powershell/module/appx/invoke-commandindesktoppackage
    # PreventBreakaway is required so the helper's provider child also has identity.
    $timer = [Diagnostics.Stopwatch]::StartNew()
    Invoke-CommandInDesktopPackage -PackageFamilyName $package.PackageFamilyName `
        -AppId ([string]$manifest.Package.Applications.Application.Id) -Command $pwsh `
        -Args "-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand $encodedWrapper" -PreventBreakaway
    while (-not (Test-Path -LiteralPath $resultPath -PathType Leaf) -and $timer.Elapsed.TotalSeconds -lt 60) {
        Start-Sleep -Milliseconds 200
    }
    if (-not (Test-Path -LiteralPath $resultPath -PathType Leaf)) { throw 'Packaged activation test produced no result within 60 seconds.' }
    $result = Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json
    if ($result.token -ne $runToken -or $result.exitCode -ne 0) { throw "Packaged activation self-test failed: $($result | ConvertTo-Json -Compress)" }
    $testPassed = $true
} finally {
    try { Stop-TestProcess } catch { $cleanupErrors.Add("Process cleanup: $_") }
    foreach ($log in @($wrapperLog, (Join-Path $providerLogDirectory 'stdout.log'), (Join-Path $providerLogDirectory 'stderr.log'))) {
        Write-Host "Packaged activation test log: $log"
        if (Test-Path -LiteralPath $log -PathType Leaf) {
            try { Get-Content -LiteralPath $log -Raw | Write-Host } catch { Write-Warning "Could not read log: $_" }
        } else { Write-Host '(not created)' }
    }
    if ($registrationAttempted) {
        try {
            Get-AppxPackage -Name $packageName | Where-Object { $_.InstallLocation -eq $layout } |
                ForEach-Object { Remove-AppxPackage -Package $_.PackageFullName }
        } catch { $cleanupErrors.Add("Preview unregister: $_") }
    }
    if ($registry) {
        foreach ($name in $registryBackup.Keys) {
            try {
                $backup = $registryBackup[$name]
                if ($backup.exists) { $registry.SetValue($name, $backup.value, $backup.kind) }
                else { $registry.DeleteValue($name, $false) }
            } catch { $cleanupErrors.Add("Restore registry value ${name}: $_") }
        }
        $removeEmptyKey = -not $registryExisted -and $registry.ValueCount -eq 0 -and $registry.SubKeyCount -eq 0
        $registry.Dispose()
        if ($removeEmptyKey) {
            try { [Microsoft.Win32.Registry]::LocalMachine.DeleteSubKey($registryPath, $false) }
            catch { $cleanupErrors.Add("Remove temporary registry key: $_") }
        }
    }
    foreach ($errorText in $cleanupErrors) { Write-Warning $errorText }
}
if ($cleanupErrors.Count) { throw 'Package test cleanup failed; see warnings.' }
if (-not $testPassed) { throw 'Packaged activation test did not complete.' }
$buildInfo.providerPackagedActivationSelfTestPassed = $true
Save-BuildInfo
Write-Host 'Package identity and LOCAL_SERVER COM activation passed for a same-process temporary class. Cross-process proxy marshaling and Widgets host/display integration were not verified.'
if ($env:GITHUB_STEP_SUMMARY) {
    '- Package identity and LOCAL_SERVER COM activation passed for a same-process temporary class on the runner; preview unregistered and Developer Mode restored. Cross-process proxy marshaling and Widgets host/display integration were not verified.' |
        Out-File -LiteralPath $env:GITHUB_STEP_SUMMARY -Append -Encoding utf8
}
