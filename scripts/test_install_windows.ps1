[CmdletBinding()]
param([string]$BinaryPath = "", [string]$GatewayUrl = "")
$ErrorActionPreference = "Stop"
$installer = Join-Path $PSScriptRoot "../install.ps1"
$tokens = $null; $errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($installer, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors | Out-String) }
foreach ($name in @("Get-NormalizedPathEntry", "Get-WocliPathUpdate", "Get-WocliLatestTag")) {
    $function = $ast.Find({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name }, $true)
    if (-not $function) { throw "Missing function $name" }
    Invoke-Expression $function.Extent.Text
}
function Assert-Equal($Actual, $Expected) {
    if ($Actual -cne $Expected) { throw "Expected '$Expected', got '$Actual'" }
}
# Persistent PATH already configured, but the shell was launched with an old PATH.
$update = Get-WocliPathUpdate -UserPath 'C:\Tools\wocli' -ProcessPath 'C:\Windows' -Directory 'C:\Tools\wocli'
Assert-Equal $update.UserPath 'C:\Tools\wocli'
Assert-Equal $update.ProcessPath 'C:\Windows;C:\Tools\wocli'
# Reinstallation normalizes quoted/case/trailing-separator duplicates and preserves other entries.
$update = Get-WocliPathUpdate -UserPath '"c:\tools\wocli\";C:\Other;C:\Tools\wocli' -ProcessPath 'C:\Windows;C:\Tools\wocli\' -Directory 'C:\Tools\wocli'
Assert-Equal $update.UserPath 'C:\Other;C:\Tools\wocli'
Assert-Equal $update.ProcessPath 'C:\Windows;C:\Tools\wocli'
$update = Get-WocliPathUpdate -Directory 'C:\Tools\wocli'
Assert-Equal $update.UserPath 'C:\Tools\wocli'
Assert-Equal $update.ProcessPath 'C:\Tools\wocli'
$update = Get-WocliPathUpdate -UserPath 'C:\Other;C:\Tools\wocli' -ProcessPath 'C:\Tools\wocli;C:\Windows' -Directory 'C:\Tools\wocli' -Remove
Assert-Equal $update.UserPath 'C:\Other'
Assert-Equal $update.ProcessPath 'C:\Windows'
Write-Host 'PATH normalization and stale-shell regression tests passed.'

# On Windows, exercise the complete installer twice with an offline release fixture.
if ($BinaryPath) {
    if ($env:OS -ne 'Windows_NT') { throw 'Binary installation test requires Windows' }
    $binary = (Resolve-Path $BinaryPath).Path
    $temp = Join-Path ([IO.Path]::GetTempPath()) "wocli-installer-test-$([Guid]::NewGuid())"
    $savedUserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $savedProcessPath = $env:Path
    $savedHome = $env:WOCLI_HOME
    $savedRuntime = $env:WOCLI_RUNTIME_DIR
    $savedAudio = $env:WOCLI_AUDIO_OUTPUT
    $savedDownloadBase = $env:WOCLI_DOWNLOAD_BASE
    $savedLocalAppData = $env:LOCALAPPDATA
    try {
        $env:WOCLI_HOME = Join-Path $temp 'test-data'
        $env:WOCLI_RUNTIME_DIR = Join-Path $temp 'test-run'
        $env:WOCLI_AUDIO_OUTPUT = 'null'

        $version = ((Get-Content (Join-Path $PSScriptRoot '../Cargo.toml')) | Select-String '^version = "([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
        $global:WocliInstallerFixtureTag = "v$version"
        $name = "wocli-music-$version-windows-x86_64"
        $bundle = Join-Path $temp $name
        New-Item -ItemType Directory -Path $bundle -Force | Out-Null
        Copy-Item $binary (Join-Path $bundle 'wocli.exe')
        $global:WocliInstallerFixtureZip = Join-Path $temp "$name.zip"
        $global:WocliInstallerFixtureSums = Join-Path $temp 'SHA256SUMS'
        Compress-Archive -Path $bundle -DestinationPath $global:WocliInstallerFixtureZip
        "$((Get-FileHash $global:WocliInstallerFixtureZip -Algorithm SHA256).Hash.ToLower())  $name.zip" | Set-Content $global:WocliInstallerFixtureSums
        function global:curl.exe {
            # Match native curl argv literally; PowerShell parameter binding must
            # not reinterpret -o as a named parameter abbreviation.
            if ($args.Count -ne 4 -or $args[0] -ne '-fL' -or $args[2] -ne '-o') { throw 'Unexpected fixture curl arguments' }
            $url = [string]$args[1]
            $outputPath = [string]$args[3]
            $expectedBase = if ($env:WOCLI_DOWNLOAD_BASE -eq 'github') { 'https://github.com/yhan-sun/wocli-music/releases/download/' } else { 'https://wocli.yhan.fun/releases/download/' }
            if (-not $url.StartsWith($expectedBase)) { throw "Unexpected distribution URL: $url" }
            if ($url.EndsWith('/SHA256SUMS')) { Copy-Item $global:WocliInstallerFixtureSums $outputPath }
            else { Copy-Item $global:WocliInstallerFixtureZip $outputPath }
            $global:LASTEXITCODE = 0
        }
        $destination = Join-Path $temp 'installed player'
        # Reproduce a stale terminal before *both* the first install and reinstall.
        [Environment]::SetEnvironmentVariable('Path', "$savedUserPath;$destination", 'User')
        for ($i = 0; $i -lt 2; $i++) {
            $env:Path = $savedProcessPath
            $env:WOCLI_DOWNLOAD_BASE = if ($i -eq 0) { $null } else { 'github' }
            & $installer -InstallDir $destination -Version "v$version"
            Assert-Equal (Get-Command wocli -CommandType Application | Select-Object -First 1).Source (Join-Path $destination 'wocli.exe')
            & wocli --version
            if ($LASTEXITCODE -ne 0) { throw 'Installed command failed' }
        }
        Write-Host 'Offline Windows install/reinstall command discovery tests passed.'
        if ($GatewayUrl) {
            # Run the real installer metadata function with the actual PS web
            # cmdlet before introducing fixture transport. This catches reserved
            # header failures specific to Windows PowerShell 5.1.
            $liveReleaseTag = Get-WocliLatestTag -Uri 'https://wocli.yhan.fun/api/releases/latest'
            if ($liveReleaseTag -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+$') { throw 'Live metadata lookup failed' }
            Write-Host "PASS: real PowerShell metadata lookup returned $liveReleaseTag"
            # Fetch the real HTTP script unchanged. Only release metadata/archive
            # transport is mocked so this test also works before the next release.
            function Invoke-RestMethod {
                [CmdletBinding()]
                param([Parameter(Position=0)][string]$Uri, [switch]$UseBasicParsing,
                    [int]$TimeoutSec, [hashtable]$Headers, [string]$UserAgent)
                if ($Uri -eq 'https://wocli.yhan.fun/api/releases/latest') {
                    if ($UserAgent -ne 'wocli-installer' -or ($Headers -and $Headers.ContainsKey('User-Agent'))) { throw 'Invalid metadata UserAgent parameter' }
                    return @{ tag_name = $global:WocliInstallerFixtureTag }
                }
                $text = Microsoft.PowerShell.Utility\Invoke-RestMethod @PSBoundParameters
                if ($Uri -eq $GatewayUrl) {
                    if ($text -isnot [string]) { throw 'HTTP installer must be decoded script text' }
                    if ($text[0] -eq [char]0xFEFF) { throw 'HTTP installer contains a literal BOM; irm | iex cannot parse it' }
                }
                return $text
            }
            $env:LOCALAPPDATA = Join-Path $temp 'gateway-profile'
            $env:WOCLI_DOWNLOAD_BASE = $null
            $env:Path = $savedProcessPath
            $gatewayExpectedExe = Join-Path $env:LOCALAPPDATA 'Programs/wocli-music/wocli.exe'
            # This is the README one-liner, evaluated with a real HTTP response.
            irm $GatewayUrl | iex
            Assert-Equal (Get-Command wocli -CommandType Application | Select-Object -First 1).Source ([IO.Path]::GetFullPath($gatewayExpectedExe))
            & wocli --version
            if ($LASTEXITCODE -ne 0) { throw 'HTTP-installed command failed' }
            # Also verify the documented custom-directory ScriptBlock invocation.
            $gatewayCustomDir = Join-Path $temp 'gateway custom install'
            & ([scriptblock]::Create((irm $GatewayUrl))) -InstallDir $gatewayCustomDir -Version $global:WocliInstallerFixtureTag
            if (-not (Test-Path (Join-Path $gatewayCustomDir 'wocli.exe'))) { throw 'HTTP ScriptBlock installation failed' }
            Write-Host 'PASS: live irm | iex and ScriptBlock installation with immediate command availability.'
        }
    } finally {
        Remove-Item Function:\curl.exe -ErrorAction SilentlyContinue
        if ($GatewayUrl) { Remove-Item Function:\Invoke-RestMethod -ErrorAction SilentlyContinue }
        Remove-Variable WocliInstallerFixtureZip, WocliInstallerFixtureSums, WocliInstallerFixtureTag -Scope Global -ErrorAction SilentlyContinue
        [Environment]::SetEnvironmentVariable('Path', $savedUserPath, 'User')
        $env:Path = $savedProcessPath
        $env:WOCLI_HOME = $savedHome
        $env:WOCLI_RUNTIME_DIR = $savedRuntime
        $env:WOCLI_AUDIO_OUTPUT = $savedAudio
        $env:WOCLI_DOWNLOAD_BASE = $savedDownloadBase
        $env:LOCALAPPDATA = $savedLocalAppData
        Remove-Item $temp -Recurse -Force -ErrorAction SilentlyContinue
    }
}
