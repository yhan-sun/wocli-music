#requires -Version 7.4
# Build-only preview packaging. This script never installs a package/certificate,
# enables Developer Mode, starts the live provider, or changes Widgets settings.
[CmdletBinding()]
param(
    [ValidateSet('x64', 'arm64')][string]$Architecture = 'x64',
    [ValidateSet('Release', 'Debug')][string]$Configuration = 'Release',
    [string]$PackageVersion = '',
    [string]$OutputDirectory = '',
    [string]$PlayerBinary = '',
    [string]$NativeBundleDirectory = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'Build this preview on Windows with .NET 8 SDK and Windows SDK 22621 or newer.' }
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$project = Join-Path $repo 'windows/widgets/Wocli.Widgets/Wocli.Widgets.csproj'
$coreTests = Join-Path $repo 'windows/widgets/Wocli.Widgets.Core.Tests/Wocli.Widgets.Core.Tests.csproj'
$sourceManifest = Join-Path $repo 'windows/widgets/Package.appxmanifest'
$dotnet = (Get-Command dotnet -CommandType Application -ErrorAction Stop).Source
Add-Type -AssemblyName System.Drawing
if ($PlayerBinary) {
    $PlayerBinary = (Resolve-Path -LiteralPath $PlayerBinary -ErrorAction Stop).Path
    if (-not (Test-Path -LiteralPath $PlayerBinary -PathType Leaf)) { throw 'PlayerBinary must be a file.' }
    if (-not $NativeBundleDirectory) { throw 'PlayerBinary requires -NativeBundleDirectory from scripts/build_native.py so dependency notices accompany the player.' }
    $NativeBundleDirectory = (Resolve-Path -LiteralPath $NativeBundleDirectory -ErrorAction Stop).Path
    foreach ($required in @('licenses', 'licenses/TaskbarLyrics/LICENSE', 'THIRD_PARTY.txt')) {
        if (-not (Test-Path -LiteralPath (Join-Path $NativeBundleDirectory $required))) { throw "Missing native player notice: $required" }
    }
}

function Invoke-CheckedNative {
    param([string]$Executable, [string[]]$Arguments)
    & $Executable @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Native command failed ($LASTEXITCODE): $Executable $($Arguments -join ' ')" }
}

function Find-MakeAppx {
    $existing = Get-Command makeappx.exe -CommandType Application -ErrorAction SilentlyContinue
    if ($existing) { return $existing.Source }
    $sdkBin = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10/bin'
    $toolArchitecture = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'arm64' } else { 'x64' }
    $sdkVersions = Get-ChildItem -LiteralPath $sdkBin -Directory |
        Where-Object { $_.Name -match '^10\.0\.\d+\.\d+$' -and [version]$_.Name -ge [version]'10.0.22621.0' } |
        Sort-Object { [version]$_.Name } -Descending
    foreach ($sdkVersion in $sdkVersions) {
        $candidate = Join-Path $sdkVersion.FullName "$toolArchitecture/makeappx.exe"
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
    }
    throw 'MakeAppx.exe not found. Install Windows SDK 10.0.22621.0 or newer.'
}

function Export-MusicMark {
    param([string]$Path, [int]$Size, [Drawing.Color]$Color)
    # Export the existing src/tray.rs U+266B/Segoe UI Symbol mark at build time.
    # These are image assets only; the provider never draws taskbar text or UI.
    $bitmap = [Drawing.Bitmap]::new($Size, $Size, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = $null
    $font = $null
    $brush = $null
    $format = $null
    try {
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        $graphics.Clear([Drawing.Color]::Transparent)
        $graphics.TextRenderingHint = [Drawing.Text.TextRenderingHint]::AntiAliasGridFit
        $font = [Drawing.Font]::new('Segoe UI Symbol', [single]($Size * 0.69), [Drawing.FontStyle]::Bold, [Drawing.GraphicsUnit]::Pixel)
        $brush = [Drawing.SolidBrush]::new($Color)
        $format = [Drawing.StringFormat]::new()
        $format.Alignment = [Drawing.StringAlignment]::Center
        $format.LineAlignment = [Drawing.StringAlignment]::Center
        $graphics.DrawString([string][char]0x266B, $font, $brush, [Drawing.RectangleF]::new(0, 0, $Size, $Size), $format)
        $bitmap.Save($Path, [Drawing.Imaging.ImageFormat]::Png)
    } finally {
        if ($format) { $format.Dispose() }
        if ($brush) { $brush.Dispose() }
        if ($font) { $font.Dispose() }
        if ($graphics) { $graphics.Dispose() }
        $bitmap.Dispose()
    }
}

function Export-WidgetPickerPreview {
    param([string]$Path, [bool]$Dark)
    # The picker requires a 300x304 medium-card preview with transparent corners.
    # This deterministic illustrative asset is NOT a captured Windows UI test.
    # Runtime card text, colors and layout are rendered by the Windows host.
    $bitmap = [Drawing.Bitmap]::new(300, 304, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $resources = [Collections.Generic.List[IDisposable]]::new()
    try {
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        $resources.Add($graphics)
        $graphics.Clear([Drawing.Color]::Transparent)
        $graphics.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $graphics.TextRenderingHint = [Drawing.Text.TextRenderingHint]::AntiAliasGridFit
        $background = [Drawing.SolidBrush]::new($(if ($Dark) { [Drawing.Color]::FromArgb(255, 32, 32, 32) } else { [Drawing.Color]::FromArgb(255, 250, 250, 250) }))
        $foreground = [Drawing.SolidBrush]::new($(if ($Dark) { [Drawing.Color]::FromArgb(255, 245, 245, 245) } else { [Drawing.Color]::FromArgb(255, 30, 30, 30) }))
        $subtle = [Drawing.SolidBrush]::new($(if ($Dark) { [Drawing.Color]::FromArgb(255, 192, 192, 192) } else { [Drawing.Color]::FromArgb(255, 95, 95, 95) }))
        $resources.Add($background); $resources.Add($foreground); $resources.Add($subtle)
        $rounded = [Drawing.Drawing2D.GraphicsPath]::new()
        $resources.Add($rounded)
        $rounded.AddArc(0, 0, 24, 24, 180, 90)
        $rounded.AddArc(275, 0, 24, 24, 270, 90)
        $rounded.AddArc(275, 279, 24, 24, 0, 90)
        $rounded.AddArc(0, 279, 24, 24, 90, 90)
        $rounded.CloseFigure()
        $graphics.FillPath($background, $rounded)
        $normal = [Drawing.Font]::new('Microsoft YaHei UI', 14, [Drawing.FontStyle]::Regular, [Drawing.GraphicsUnit]::Pixel)
        $heading = [Drawing.Font]::new('Microsoft YaHei UI', 19, [Drawing.FontStyle]::Bold, [Drawing.GraphicsUnit]::Pixel)
        $lyric = [Drawing.Font]::new('Microsoft YaHei UI', 20, [Drawing.FontStyle]::Regular, [Drawing.GraphicsUnit]::Pixel)
        $caption = [Drawing.Font]::new('Microsoft YaHei UI', 11, [Drawing.FontStyle]::Regular, [Drawing.GraphicsUnit]::Pixel)
        $resources.Add($normal); $resources.Add($heading); $resources.Add($lyric); $resources.Add($caption)
        $graphics.DrawString('wocli 歌词', $normal, $foreground, [single]20, [single]18)
        $graphics.DrawString('示例歌曲', $heading, $foreground, [single]20, [single]64)
        $graphics.DrawString('示例歌手', $normal, $subtle, [single]20, [single]96)
        $graphics.DrawString('此处显示当前歌词', $lyric, $foreground, [Drawing.RectangleF]::new(20, 142, 260, 72))
        $graphics.DrawString('正在播放', $normal, $subtle, [single]20, [single]226)
        $graphics.DrawString('卡片示意 · 实际样式由 Windows 决定', $caption, $subtle, [single]20, [single]273)
        $bitmap.Save($Path, [Drawing.Imaging.ImageFormat]::Png)
    } finally {
        foreach ($resource in $resources) { $resource.Dispose() }
        $bitmap.Dispose()
    }
}

function Test-WidgetManifestResources {
    param([xml]$Manifest, [string]$Layout)
    # MakeAppx only checks that app-extension properties are well-formed XML.
    # Validate the actual widget-host contract and every picker resource too.
    $extension = @($Manifest.SelectNodes('//*[local-name()="AppExtension" and @Name="com.microsoft.windows.widgets"]'))
    if ($extension.Count -ne 1) { throw 'Expected exactly one ordinary Windows Widgets extension.' }
    $provider = $extension[0].SelectSingleNode('*[local-name()="Properties"]/*[local-name()="WidgetProvider"]')
    if (-not $provider -or $provider.Activation.CreateInstance.ClassId -ne '8B3EF0C4-86E8-4D42-A154-76492840B5EB') {
        throw 'Missing WidgetProvider COM activation.'
    }
    $definition = $provider.Definitions.Definition
    if ($definition.Id -ne 'Wocli_Lyrics' -or $definition.AllowMultiple -ne 'false') { throw 'Incorrect lyrics widget definition.' }
    foreach ($attribute in @('ExclusiveRegions', 'ExcludedRegions', 'ContentUri', 'WebRequestFilter')) {
        if ($definition.HasAttribute($attribute)) { throw "Unexpected Feed/web/region field in Adaptive Card registration: $attribute" }
    }
    $sizes = @($definition.Capabilities.Capability | ForEach-Object { $_.Size.Name } | Sort-Object)
    if (($sizes -join ',') -ne 'large,medium,small') { throw 'All three widget sizes must be registered exactly once.' }
    $screenshots = @($definition.SelectNodes('*[local-name()="ThemeResources"]//*[local-name()="Screenshot"]'))
    if ($screenshots.Count -ne 3) { throw 'Default, light and dark picker previews are required.' }
    foreach ($resource in @($provider.SelectNodes('.//*[local-name()="Icon" or local-name()="Screenshot"]'))) {
        $path = [string]$resource.Path
        if ($path -notmatch '^Assets\\[A-Za-z0-9]+\.png$') { throw "Invalid widget resource path: $path" }
        $fullPath = Join-Path $Layout $path
        if (-not (Test-Path -LiteralPath $fullPath -PathType Leaf)) { throw "Missing widget resource: $path" }
        if ($resource.LocalName -eq 'Screenshot') {
            $preview = [Drawing.Bitmap]::new($fullPath)
            try {
                if ($preview.Width -ne 300 -or $preview.Height -ne 304 -or
                    $preview.GetPixel(0, 0).A -ne 0 -or $preview.GetPixel(299, 303).A -ne 0) {
                    throw "Picker preview must be 300x304 with transparent rounded corners: $path"
                }
            } finally { $preview.Dispose() }
        }
    }
}

function Read-PackageManifest {
    param([string]$Path)
    $archive = [IO.Compression.ZipFile]::OpenRead($Path)
    try {
        $entry = $archive.GetEntry('AppxManifest.xml')
        if (-not $entry) { throw "Missing AppxManifest.xml in $Path" }
        $reader = [IO.StreamReader]::new($entry.Open())
        try { return [xml]$reader.ReadToEnd() } finally { $reader.Dispose() }
    } finally { $archive.Dispose() }
}

function Find-RestoredPackage {
    param($Assets, [string]$Name)
    $library = $Assets.libraries.PSObject.Properties | Where-Object { $_.Name.Split('/')[0] -ieq $Name } | Select-Object -First 1
    if (-not $library) { throw "Package not found in restore assets: $Name" }
    foreach ($packageFolder in $Assets.packageFolders.PSObject.Properties.Name) {
        $candidate = Join-Path $packageFolder $library.Value.path
        if (Test-Path -LiteralPath $candidate -PathType Container) { return $candidate }
    }
    throw "Restored package directory not found: $Name"
}

function Export-DotnetNotices {
    param($Assets, $RuntimeConfig, [string]$Destination)
    $packagePaths = [Collections.Generic.Dictionary[string, string]]::new()
    foreach ($library in $Assets.libraries.PSObject.Properties) {
        if ($library.Value.type -ne 'package') { continue }
        foreach ($packageFolder in $Assets.packageFolders.PSObject.Properties.Name) {
            $candidate = Join-Path $packageFolder $library.Value.path
            if (Test-Path -LiteralPath $candidate -PathType Container) {
                $packagePaths[$library.Name] = $candidate
                break
            }
        }
    }
    # Runtime packs can be restore download-dependencies rather than entries in
    # libraries. Derive their exact version from the actual published app.
    foreach ($framework in $RuntimeConfig.runtimeOptions.includedFrameworks) {
        $relative = "$($framework.name.ToLowerInvariant()).runtime.win-$Architecture/$($framework.version)"
        foreach ($packageFolder in $Assets.packageFolders.PSObject.Properties.Name) {
            $candidate = Join-Path $packageFolder $relative
            if (Test-Path -LiteralPath $candidate -PathType Container) {
                $packagePaths[$relative] = $candidate
                break
            }
        }
        if (-not $packagePaths.ContainsKey($relative)) { throw "Cannot locate license-bearing .NET runtime pack: $relative" }
    }
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    $inventory = [Collections.Generic.List[string]]::new()
    $inventory.Add('Resolved NuGet packages and .NET runtime packs; notices apply to their respective third-party components.')
    foreach ($entry in ($packagePaths.GetEnumerator() | Sort-Object Key)) {
        $inventory.Add($entry.Key)
        $notices = @(Get-ChildItem -LiteralPath $entry.Value -Recurse -File | Where-Object {
            $_.Name -match '^(?i:licen[cs]e|copying|notice|third.?party.*notices)' -and $_.Extension -notin @('.dll', '.exe', '.pdb')
        })
        foreach ($notice in $notices) {
            $relative = [IO.Path]::GetRelativePath($entry.Value, $notice.FullName)
            $outputPath = Join-Path (Join-Path $Destination $entry.Key) $relative
            New-Item -ItemType Directory -Path (Split-Path $outputPath -Parent) -Force | Out-Null
            Copy-Item -LiteralPath $notice.FullName -Destination $outputPath
        }
        # .NET runtime packs contain both license and third-party notices.
        if ($entry.Key -like 'microsoft.netcore.app.runtime.*' -and $notices.Count -lt 2) {
            throw "Incomplete .NET runtime notices for $($entry.Key)"
        }
    }
    $inventory | Set-Content -LiteralPath (Join-Path $Destination 'PACKAGES.txt') -Encoding utf8
}

if (-not $PackageVersion) {
    # Preview revisions need distinct package identities for upgrades even when
    # their Cargo major/minor/patch is unchanged (alpha.1 => .0, alpha.2 => .1).
    [xml]$versionManifest = Get-Content -LiteralPath $sourceManifest -Raw
    $PackageVersion = [string]$versionManifest.Package.Identity.Version
}
if ($PackageVersion -notmatch '^\d+\.\d+\.\d+\.\d+$' -or
    @($PackageVersion.Split('.') | Where-Object { [uint64]$_ -gt 65535 }).Count) {
    throw 'PackageVersion must have four integer components between 0 and 65535.'
}
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $repo 'artifacts/system-widgets-preview' }
$outputRoot = [IO.Path]::GetFullPath($OutputDirectory)
$destination = Join-Path $outputRoot $Architecture
# A previous Layout may be registered with Windows. Never delete it underneath
# a running/registered provider merely because a developer starts another build.
if (Test-Path -LiteralPath $destination) {
    throw "Output already exists: $destination. Choose another -OutputDirectory, or unregister and remove the old preview first."
}
$makeAppx = Find-MakeAppx
New-Item -ItemType Directory -Path $outputRoot -Force | Out-Null
$staging = Join-Path $outputRoot ('.staging-' + $Architecture + '-' + [Guid]::NewGuid().ToString('N'))
$layout = Join-Path $staging 'Layout'
New-Item -ItemType Directory -Path $layout -Force | Out-Null

try {
    Invoke-CheckedNative $dotnet @('run', '--project', $coreTests, '--configuration', $Configuration)
    $platform = if ($Architecture -eq 'arm64') { 'ARM64' } else { 'x64' }
    Invoke-CheckedNative $dotnet @(
        'publish', $project, '--configuration', $Configuration,
        '--runtime', "win-$Architecture", '--self-contained', 'true', '--output', $layout,
        "-p:Platform=$platform", '-p:WindowsAppSDKSelfContained=false',
        '-p:PublishSingleFile=false', '-p:PublishTrimmed=false', '-p:DebugType=None', '-p:DebugSymbols=false'
    )

    # Self-contained means the payload carries .NET, not merely a runtimeconfig
    # that asks the tester to install it globally.
    foreach ($file in @('Wocli.Widgets.exe', 'Wocli.Widgets.dll', 'Wocli.Widgets.runtimeconfig.json', 'coreclr.dll', 'hostfxr.dll')) {
        if (-not (Test-Path -LiteralPath (Join-Path $layout $file) -PathType Leaf)) { throw "Missing self-contained payload file: $file" }
    }
    $runtimeConfig = Get-Content -LiteralPath (Join-Path $layout 'Wocli.Widgets.runtimeconfig.json') -Raw | ConvertFrom-Json
    if ($runtimeConfig.runtimeOptions.PSObject.Properties.Name -contains 'framework' -or
        $runtimeConfig.runtimeOptions.PSObject.Properties.Name -contains 'frameworks') {
        throw 'The published provider still requires a shared .NET framework.'
    }
    $restoreAssets = Get-Content -LiteralPath (Join-Path (Split-Path $project -Parent) 'obj/project.assets.json') -Raw | ConvertFrom-Json
    $runtimePackage = Find-RestoredPackage $restoreAssets 'Microsoft.WindowsAppSDK.Runtime'
    $runtimeCandidates = @(Get-ChildItem -LiteralPath (Join-Path $runtimePackage "tools/MSIX/win10-$Architecture") -Filter '*.msix' -File |
        Where-Object { (Read-PackageManifest $_.FullName).SelectSingleNode('/*[local-name()="Package"]/*[local-name()="Properties"]/*[local-name()="Framework" and text()="true"]') })
    if ($runtimeCandidates.Count -ne 1) { throw 'Expected exactly one official Windows App Runtime framework for the target architecture.' }
    $runtimeMsix = $runtimeCandidates[0].FullName
    $runtimeManifest = Read-PackageManifest $runtimeMsix
    $runtimeIdentity = $runtimeManifest.Package.Identity
    if ($runtimeIdentity.ProcessorArchitecture -ne $Architecture -or $runtimeManifest.Package.Properties.Framework -ne 'true') {
        throw 'The bundled Windows App SDK package is not the expected architecture/framework.'
    }
    $runtimeSignature = Get-AuthenticodeSignature -LiteralPath $runtimeMsix
    if ($runtimeSignature.Status -ne 'Valid' -or $runtimeSignature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation(?:,|$)') {
        throw 'Windows App SDK framework must carry a valid Microsoft signature.'
    }
    $dependencies = Join-Path $staging 'Dependencies'
    New-Item -ItemType Directory -Path $dependencies -Force | Out-Null
    $runtimeFilename = "$($runtimeIdentity.Name)_$($runtimeIdentity.Version)_$Architecture.msix"
    Copy-Item -LiteralPath $runtimeMsix -Destination (Join-Path $dependencies $runtimeFilename)
    # Never silently omit another framework needed by the actual selected MSIX.
    # Fail with its identity instead of producing a package that cannot register.
    $runtimeDependencies = @($runtimeManifest.SelectNodes('/*[local-name()="Package"]/*[local-name()="Dependencies"]/*[local-name()="PackageDependency"]'))
    if ($runtimeDependencies.Count -ne 0) {
        throw "The selected Windows App SDK framework has additional package dependencies that must be bundled: $($runtimeDependencies.OuterXml -join ', ')"
    }
    Export-DotnetNotices $restoreAssets $runtimeConfig (Join-Path $layout 'licenses')
    # This in-process COM round-trip checks our interfaces without installing the package,
    # opening a Widgets host or connecting to a user's player daemon.
    # Cross-architecture execution must be tested on its own native runner.
    $nativeArchitecture = [Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString().ToLowerInvariant()
    $selfTestPassed = $false
    if ($nativeArchitecture -eq $Architecture) {
        & (Join-Path $PSScriptRoot 'test_windows_widgets.ps1') -ProviderPath (Join-Path $layout 'Wocli.Widgets.exe')
        $selfTestPassed = $true
    } else {
        Write-Warning "Provider self-test requires a native $Architecture runner; this build only type-checks it."
    }

    $assets = Join-Path $layout 'Assets'
    New-Item -ItemType Directory -Path $assets -Force | Out-Null
    Export-MusicMark (Join-Path $assets 'StoreLogo.png') 50 ([Drawing.Color]::FromArgb(255, 255, 215, 0))
    Export-MusicMark (Join-Path $assets 'Square44x44Logo.png') 44 ([Drawing.Color]::FromArgb(255, 255, 215, 0))
    Export-MusicMark (Join-Path $assets 'Square150x150Logo.png') 150 ([Drawing.Color]::FromArgb(255, 255, 215, 0))
    Export-MusicMark (Join-Path $assets 'WidgetLight.png') 64 ([Drawing.Color]::FromArgb(255, 32, 32, 32))
    Export-MusicMark (Join-Path $assets 'WidgetDark.png') 64 ([Drawing.Color]::FromArgb(255, 240, 240, 240))
    Export-WidgetPickerPreview (Join-Path $assets 'WidgetPreviewLight.png') $false
    Export-WidgetPickerPreview (Join-Path $assets 'WidgetPreviewDark.png') $true
    Copy-Item -LiteralPath (Join-Path $repo 'windows/widgets/Assets/README.txt') -Destination $assets
    $public = Join-Path $layout 'Public'
    New-Item -ItemType Directory -Path $public -Force | Out-Null
    'The Widgets host renders local Adaptive Card data; this provider supplies no HTML or web resources.' |
        Set-Content -LiteralPath (Join-Path $public 'README.txt') -Encoding utf8

    [xml]$manifest = Get-Content -LiteralPath $sourceManifest -Raw
    $manifest.Package.Identity.SetAttribute('ProcessorArchitecture', $Architecture)
    $manifest.Package.Identity.SetAttribute('Version', $PackageVersion)
    # Exact identity and minimum version come from the signed framework package,
    # not a guessed version mapping from the SDK's NuGet version number.
    $dependency = $manifest.CreateElement('PackageDependency', $manifest.DocumentElement.NamespaceURI)
    $dependency.SetAttribute('Name', [string]$runtimeIdentity.Name)
    $dependency.SetAttribute('Publisher', [string]$runtimeIdentity.Publisher)
    $dependency.SetAttribute('MinVersion', [string]$runtimeIdentity.Version)
    [void]$manifest.Package.Dependencies.AppendChild($dependency)
    Test-WidgetManifestResources $manifest $layout
    $manifest.Save((Join-Path $layout 'AppxManifest.xml'))
    $packageName = "wocli-system-widgets-preview-$PackageVersion-$Architecture.msix"
    $packagePath = Join-Path $staging $packageName
    # Do not pass /nv: MakeAppx must perform its manifest and semantic validation.
    Invoke-CheckedNative $makeAppx @('pack', '/d', $layout, '/p', $packagePath, '/o', '/v')

    $payload = @(Get-ChildItem -LiteralPath $layout -Recurse -File)
    $payloadBytes = [long](($payload | Measure-Object -Property Length -Sum).Sum)
    $packageBytes = (Get-Item -LiteralPath $packagePath).Length
    $sha256 = (Get-FileHash -LiteralPath $packagePath -Algorithm SHA256).Hash.ToLowerInvariant()
    "$sha256  $packageName" | Set-Content -LiteralPath (Join-Path $staging 'SHA256SUMS') -Encoding utf8
    $runtimeSha256 = (Get-FileHash -LiteralPath $runtimeMsix -Algorithm SHA256).Hash.ToLowerInvariant()
    "$runtimeSha256  Dependencies/$runtimeFilename" | Add-Content -LiteralPath (Join-Path $staging 'SHA256SUMS') -Encoding utf8
    if ($PlayerBinary) {
        Copy-Item -LiteralPath $PlayerBinary -Destination (Join-Path $staging 'wocli.exe')
        Copy-Item -LiteralPath (Join-Path $repo 'NOTICE.md') -Destination $staging
        Copy-Item -LiteralPath (Join-Path $NativeBundleDirectory 'licenses') -Destination (Join-Path $staging 'licenses') -Recurse
        Copy-Item -LiteralPath (Join-Path $NativeBundleDirectory 'THIRD_PARTY.txt') -Destination $staging
        if (Test-Path -LiteralPath (Join-Path $repo 'LICENSE') -PathType Leaf) {
            Copy-Item -LiteralPath (Join-Path $repo 'LICENSE') -Destination $staging
        }
        $playerSha256 = (Get-FileHash -LiteralPath $PlayerBinary -Algorithm SHA256).Hash.ToLowerInvariant()
        "$playerSha256  wocli.exe" | Add-Content -LiteralPath (Join-Path $staging 'SHA256SUMS') -Encoding utf8
    }
    $buildInfo = [ordered]@{
        name = 'system-widgets-preview'
        version = $PackageVersion
        architecture = $Architecture
        configuration = $Configuration
        sourceCommit = $env:GITHUB_SHA
        dotnetSdk = (& $dotnet --version | Out-String).Trim()
        windowsAppSdk = ($restoreAssets.libraries.PSObject.Properties.Name | Where-Object { $_ -like 'Microsoft.WindowsAppSDK/*' }).Split('/')[1]
        displaySurface = 'Windows Widgets board Adaptive Card; no taskbar announcements'
        regionRestricted = $false
        pickerPreviewIsIllustration = $true
        widgetsHostUiVerified = $false
        selfContainedDotnet = $true
        selfContainedWindowsAppSdk = $false
        windowsAppSdkDeployment = 'framework-dependent'
        runtimeFramework = [ordered]@{
            name = [string]$runtimeIdentity.Name
            publisher = [string]$runtimeIdentity.Publisher
            version = [string]$runtimeIdentity.Version
            package = "Dependencies/$runtimeFilename"
            bytes = (Get-Item -LiteralPath $runtimeMsix).Length
            sha256 = $runtimeSha256
            microsoftSignatureVerified = $true
        }
        signed = $false
        developmentPlayerIncluded = [bool]$PlayerBinary
        providerComSelfTestPassed = $selfTestPassed
        providerComSelfTestScope = 'in-process COM factory and interfaces; no packaged proxy or Widgets host test'
        providerPackagedProxySelfTestPassed = $false
        packageBytes = $packageBytes
        layoutBytes = $payloadBytes
        layoutFileCount = $payload.Count
        sha256 = $sha256
        makeAppx = $makeAppx
    }
    $buildInfo | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $staging 'build-info.json') -Encoding utf8
    @"
wocli system Widgets preview ($Architecture)

This is an unsigned development preview, not a stable release or an installer.
Core tests, provider publication and MakeAppx validation passed. This does not
prove that the Widgets host discovers or renders the card on the target PC.
This provider implements Microsoft's ordinary IWidgetProvider Adaptive Card API.
Open the Windows 11 Widgets board and add "wocli 歌词" to view current lyrics.
Windows renders the card's typography, colors and controls. There is no Feed,
HTML/web widget, taskbar announcement, Explorer hook or taskbar painting here.
The card manifest has no region restriction, including for China. Feed-only EEA
restrictions do not apply to this route. A supported, enabled and updated Windows
Widgets host is still required; the user's China-region device is not UI-tested.
Picker preview images are illustrative assets, not captured host screenshots.
The basic COM self-test checks the in-process factory and interfaces only.
See providerPackagedProxySelfTestPassed in build-info.json for the separate
CI-only package-identity and LOCAL_SERVER proxy-activation test result.
The matching player must come from this same development branch. Stable 0.4.10
does not implement the required lyrics_snapshot IPC or wocli widgets commands.
The included player reports 0.4.11-alpha.2, an explicitly experimental build.
It is not the published stable binary. See sourceCommit in build-info.json.

MSIX: $packageName
MSIX bytes: $packageBytes
Expanded Layout bytes: $payloadBytes
SHA256: $sha256

The Layout directory includes .NET; no global .NET runtime is required.
Windows App SDK uses the official Microsoft-signed framework MSIX supplied in
Dependencies. The SDK's self-contained registration excludes Widgets proxies,
so this preview deliberately retains framework deployment for COM activation.
The Layout must remain in its final location for as long as it is registered.
Do not run the provider EXE directly to register COM; package identity is needed.

If Developer Mode is already enabled and you choose to register this preview,
run these commands explicitly from this directory in PowerShell, in this order:
    Add-AppxPackage -Path .\Dependencies\$runtimeFilename
    Add-AppxPackage -Register (Resolve-Path .\Layout\AppxManifest.xml).Path

To remove that development registration:
    Get-AppxPackage -Name Wocli.Music.WidgetsPreview | Remove-AppxPackage

Do not install the unsigned MSIX by double-clicking it. Signed distribution
requires a separate review and matching publisher certificate. This build did
not create/trust certificates, enable Developer Mode, change region, pin a widget
or replace the weather entry. After registration, start the matching player,
run "wocli widgets on", open Widgets (Win+W), and add "wocli 歌词" in its picker.
The lyrics appear inside the expanded board, not on the collapsed weather entry.
If upgrading from alpha.1, remove its development registration first, keep this
new Layout in a permanent directory, then register it using the commands above.
The shared LocalAppData\wocli-music configuration and diagnostics are excluded
from MSIX write virtualization so the unpackaged player can read them. These
shared files remain after removing the package; other app folders are unaffected.
The Microsoft runtime framework is shared by other applications; removing this
preview does not remove that dependency. Third-party notices are in Layout/licenses
and, when the player is included, the top-level licenses and THIRD_PARTY.txt.
CI also includes the matching player dependency -sources.zip archive.
"@ | Set-Content -LiteralPath (Join-Path $staging 'PREVIEW.txt') -Encoding utf8

    Move-Item -LiteralPath $staging -Destination $destination
    Write-Host "Preview created: $destination"
    Write-Host ("Unsigned MSIX: {0:N2} MiB; expanded Layout: {1:N2} MiB ({2} files)" -f ($packageBytes / 1MB), ($payloadBytes / 1MB), $payload.Count)
    if ($env:GITHUB_STEP_SUMMARY) {
        @"
### System Widgets preview ($Architecture)

- Core tests, widget manifest/resources and MakeAppx validation passed; no host UI test was performed.
- Official Adaptive Card in the expanded Widgets board; no Feed/web widget or taskbar hook.
- Unsigned MSIX: $packageBytes bytes; expanded Layout: $payloadBytes bytes.
- SHA256: $sha256
- .NET is self-contained; the signed Windows App SDK framework MSIX is bundled separately.
- See PREVIEW.txt for dependency installation order and manual development registration.
"@ | Out-File -LiteralPath $env:GITHUB_STEP_SUMMARY -Append -Encoding utf8
    }
} finally {
    if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
}
