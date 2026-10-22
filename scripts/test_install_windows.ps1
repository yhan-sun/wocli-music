[CmdletBinding()]
param([string]$BinaryPath = "")
$ErrorActionPreference = "Stop"
$installer = Join-Path $PSScriptRoot "../install.ps1"
$tokens = $null; $errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile($installer, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors | Out-String) }
foreach ($name in @("Get-NormalizedPathEntry", "Get-WocliPathUpdate")) {
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
    try {
        $env:WOCLI_HOME = Join-Path $temp 'test-data'
        $env:WOCLI_RUNTIME_DIR = Join-Path $temp 'test-run'
        $env:WOCLI_AUDIO_OUTPUT = 'null'

        $version = ((Get-Content (Join-Path $PSScriptRoot '../Cargo.toml')) | Select-String '^version = "([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
        $name = "wocli-music-$version-windows-x86_64"
        $bundle = Join-Path $temp $name
        New-Item -ItemType Directory -Path $bundle -Force | Out-Null
        Copy-Item $binary (Join-Path $bundle 'wocli.exe')
        $script:fixtureZip = Join-Path $temp "$name.zip"
        $script:fixtureSums = Join-Path $temp 'SHA256SUMS'
        Compress-Archive -Path $bundle -DestinationPath $script:fixtureZip
        "$((Get-FileHash $script:fixtureZip -Algorithm SHA256).Hash.ToLower())  $name.zip" | Set-Content $script:fixtureSums
        function global:curl.exe {
            param($Flags, $Url, $OutputFlag, $OutputPath)
            if ($Url.EndsWith('/SHA256SUMS')) { Copy-Item $script:fixtureSums $OutputPath }
            else { Copy-Item $script:fixtureZip $OutputPath }
            $global:LASTEXITCODE = 0
        }
        $destination = Join-Path $temp 'installed player'
        # Reproduce a stale terminal before *both* the first install and reinstall.
        [Environment]::SetEnvironmentVariable('Path', "$savedUserPath;$destination", 'User')
        for ($i = 0; $i -lt 2; $i++) {
            $env:Path = $savedProcessPath
            & $installer -InstallDir $destination -Version "v$version"
            Assert-Equal (Get-Command wocli -CommandType Application | Select-Object -First 1).Source (Join-Path $destination 'wocli.exe')
            & wocli --version
            if ($LASTEXITCODE -ne 0) { throw 'Installed command failed' }
        }
        Write-Host 'Offline Windows install/reinstall command discovery tests passed.'
    } finally {
        Remove-Item Function:\curl.exe -ErrorAction SilentlyContinue
        [Environment]::SetEnvironmentVariable('Path', $savedUserPath, 'User')
        $env:Path = $savedProcessPath
        $env:WOCLI_HOME = $savedHome
        $env:WOCLI_RUNTIME_DIR = $savedRuntime
        $env:WOCLI_AUDIO_OUTPUT = $savedAudio
        Remove-Item $temp -Recurse -Force -ErrorAction SilentlyContinue
    }
}
