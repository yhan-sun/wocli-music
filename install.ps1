# wocli-music official Windows installer
# https://github.com/yhan-sun/wocli-music

[CmdletBinding()]
param (
    [string]$InstallDir = "$env:LOCALAPPDATA\Programs\wocli-music",
    [string]$Version = "",
    [switch]$Uninstall,
    [switch]$Help
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$Repo = "yhan-sun/wocli-music"
$DefaultTag = "v0.4.2"
$DownloadBase = if ($env:WOCLI_DOWNLOAD_BASE) { $env:WOCLI_DOWNLOAD_BASE.TrimEnd('/') } else { "https://wocli.yhan.fun" }
if ($DownloadBase -eq 'github') {
    $DownloadBase = "https://github.com/$Repo"
    $LatestUrl = "https://api.github.com/repos/$Repo/releases/latest"
} else { $LatestUrl = "$DownloadBase/api/releases/latest" }

function Write-Info($msg) {
    Write-Host "==> " -ForegroundColor Blue -NoNewline
    Write-Host $msg -ForegroundColor White
}

function Write-Success($msg) {
    Write-Host "==> " -ForegroundColor Green -NoNewline
    Write-Host $msg -ForegroundColor Green
}

function Write-Warn($msg) {
    Write-Host "警告: " -ForegroundColor Yellow -NoNewline
    Write-Host $msg
}

function Write-Err($msg) {
    Write-Host "错误: " -ForegroundColor Red -NoNewline
    Write-Host $msg
    exit 1
}

# Use the dedicated UserAgent parameter: Windows PowerShell 5.1 rejects
# User-Agent in -Headers before it even sends the HTTP request.
function Get-WocliLatestTag([string]$Uri) {
    $release = Invoke-RestMethod -Uri $Uri -TimeoutSec 30 -UseBasicParsing -UserAgent "wocli-installer"
    $releaseTag = [string]$release.tag_name
    if ($releaseTag -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+$') { throw "Invalid release version" }
    return $releaseTag
}

# Compare expanded, unquoted paths without changing unrelated PATH entries.
function Get-NormalizedPathEntry([string]$Entry) {
    return [Environment]::ExpandEnvironmentVariables($Entry.Trim().Trim('"')).TrimEnd('\', '/')
}

function Get-WocliPathUpdate {
    param([string]$UserPath, [string]$ProcessPath, [string]$Directory, [switch]$Remove)
    $target = Get-NormalizedPathEntry $Directory
    $updated = @()
    foreach ($value in @($UserPath, $ProcessPath)) {
        $parts = @($value -split ';' | Where-Object {
            $_ -and (Get-NormalizedPathEntry $_) -ine $target
        })
        if (-not $Remove) { $parts += $Directory }
        $updated += ($parts -join ';')
    }
    return @{ UserPath = $updated[0]; ProcessPath = $updated[1] }
}

# Tell Explorer and terminal launchers to refresh inherited environment variables.
function Send-EnvironmentChange {
    try {
        if (-not ("Wocli.EnvironmentBroadcast" -as [type])) {
            Add-Type @"
using System;
using System.Runtime.InteropServices;
namespace Wocli {
    public static class EnvironmentBroadcast {
        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        public static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint msg, UIntPtr wParam,
            string lParam, uint flags, uint timeout, out UIntPtr result);
    }
}
"@
        }
        $result = [UIntPtr]::Zero
        [void][Wocli.EnvironmentBroadcast]::SendMessageTimeout([IntPtr]0xffff, 0x001a,
            [UIntPtr]::Zero, "Environment", 2, 3000, [ref]$result)
    } catch { Write-Warn "环境变量已保存；请完全退出并重新打开终端应用以刷新 PATH。" }
}

$InstallDir = [System.IO.Path]::GetFullPath([Environment]::ExpandEnvironmentVariables($InstallDir))

if ($Help) {
    Write-Host @"
wocli-music 官方 Windows 安装脚本

用法:
  irm https://wocli.yhan.fun/install.ps1 | iex
  或: .\install.ps1 [-InstallDir <目录>] [-Version <版本>] [-Uninstall]

参数:
  -InstallDir <DIR>   指定安装目录 (默认: $env:LOCALAPPDATA\Programs\wocli-music)
  -Version <VER>      指定安装版本 (例如: v0.2.0，默认最新稳定版)
  -Uninstall          卸载 wocli-music 并清理用户 PATH 环境变量
  -Help               显示此帮助信息
"@
    exit 0
}

# 卸载逻辑
if ($Uninstall) {
    Write-Info "正在卸载 wocli-music..."

    $exePath = Join-Path $InstallDir "wocli.exe"
    if (Test-Path $exePath) {
        try {
            Write-Info "正在停止后台播放服务..."
            & $exePath daemon stop 2>$null
        } catch {}
    }

    if (Test-Path $InstallDir) {
        Remove-Item -Recurse -Force $InstallDir
        Write-Success "已删除安装目录: $InstallDir"
    }

    $pathUpdate = Get-WocliPathUpdate -UserPath ([Environment]::GetEnvironmentVariable("Path", "User")) -ProcessPath $env:Path -Directory $InstallDir -Remove
    [Environment]::SetEnvironmentVariable("Path", $pathUpdate.UserPath, "User")
    $env:Path = $pathUpdate.ProcessPath
    Send-EnvironmentChange
    Write-Success "已从用户和当前终端 PATH 中移除安装目录"

    Write-Success "wocli-music 已成功卸载。"
    Write-Host "`n注意：你的曲库数据与设置保存在 $env:APPDATA\wocli-music 中，如需彻底清理可手动删除该目录。`n"
    exit 0
}

# 平台与架构检测
$arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
if ($arch -ne "AMD64" -and $arch -ne "ARM64") {
    Write-Err "当前 Windows 架构不支持: $arch (仅支持 x86_64 / ARM64 Windows 10/11)"
}
$platform = "windows-x86_64"

# 获取版本号
$tag = $Version
if (-not $tag) {
    Write-Info "正在获取最新版本信息..."
    try {
        $tag = Get-WocliLatestTag -Uri $LatestUrl
    } catch {
        $tag = $DefaultTag
        Write-Warn "获取最新版本信息失败 ($($_.Exception.Message))，将安装默认版本 $tag"
    }
}

if ($tag -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+$') { Write-Err "版本号无效: $tag (例如 v0.3.4)" }
$cleanVer = $tag.TrimStart('v')
$archiveName = "wocli-music-$cleanVer-$platform.zip"
$downloadUrl = "$DownloadBase/releases/download/$tag/$archiveName"
$sumsUrl = "$DownloadBase/releases/download/$tag/SHA256SUMS"

$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) "wocli-install-$([System.Guid]::NewGuid().ToString().Substring(0,8))"
New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

try {
    Write-Info "目标版本: $tag"
    Write-Info "下载地址: $downloadUrl"

    $zipPath = Join-Path $tempDir $archiveName
    $sumsPath = Join-Path $tempDir "SHA256SUMS"

    if (Get-Command curl.exe -ErrorAction SilentlyContinue) {
        curl.exe -fL "$downloadUrl" -o "$zipPath"
        if ($LASTEXITCODE -ne 0) { throw "下载安装包失败 (curl: $LASTEXITCODE)" }
        curl.exe -fL "$sumsUrl" -o "$sumsPath"
        if ($LASTEXITCODE -ne 0) { throw "下载校验文件失败 (curl: $LASTEXITCODE)" }
    } else {
        Invoke-WebRequest -Uri $downloadUrl -OutFile $zipPath -UseBasicParsing
        Invoke-WebRequest -Uri $sumsUrl -OutFile $sumsPath -UseBasicParsing
    }

    Write-Info "正在校验 SHA256..."
    $hash = (Get-FileHash -Path $zipPath -Algorithm SHA256).Hash.ToLower()

    $expected = ""
    foreach ($line in (Get-Content $sumsPath)) {
        if ($line -match "^\s*([0-9a-fA-F]{64})\s+(.+)$") {
            if ($matches[2].Trim() -eq $archiveName) {
                $expected = $matches[1].ToLower()
                break
            }
        }
    }

    if (-not $expected) { throw "SHA256SUMS 中没有安装包 $archiveName 的校验值" }
    if ($hash -ne $expected) {
        Write-Err "SHA256 校验失败！`n期望: $expected`n实际: $hash"
    } elseif ($expected) {
        Write-Success "SHA256 校验通过: $hash"
    }

    Write-Info "正在解压安装..."
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    Expand-Archive -Path $zipPath -DestinationPath $tempDir -Force

    $existingExe = Join-Path $InstallDir "wocli.exe"
    if (Test-Path $existingExe) {
        Write-Info "正在停止旧版本的托盘和后台服务..."
        & $existingExe tray stop
        & $existingExe daemon stop
        if ($LASTEXITCODE -ne 0) { throw "旧版本后台未能退出，请先关闭播放器再重新安装" }
    }

    $unpackedDir = Join-Path $tempDir "wocli-music-$cleanVer-$platform"
    if (Test-Path $unpackedDir) {
        Copy-Item -Path "$unpackedDir\*" -Destination $InstallDir -Recurse -Force
    } else {
        Copy-Item -Path "$tempDir\*" -Destination $InstallDir -Recurse -Force
    }

    $binPath = Join-Path $InstallDir "wocli.exe"
    if (-not (Test-Path $binPath)) {
        Write-Err "未找到解压后的可执行文件 wocli.exe"
    }

    # Update this shell even when the persistent user PATH already contains the directory.
    $pathUpdate = Get-WocliPathUpdate -UserPath ([Environment]::GetEnvironmentVariable("Path", "User")) -ProcessPath $env:Path -Directory $InstallDir
    [Environment]::SetEnvironmentVariable("Path", $pathUpdate.UserPath, "User")
    $env:Path = $pathUpdate.ProcessPath
    Send-EnvironmentChange
    Write-Success "已配置用户和当前终端 PATH: $InstallDir"
    $command = Get-Command wocli -CommandType Application -ErrorAction Stop | Select-Object -First 1
    if ((Get-NormalizedPathEntry $command.Source) -ine (Get-NormalizedPathEntry $binPath)) {
        Write-Warn "PATH 中存在另一个 wocli，当前命令指向: $($command.Source)。本次安装可通过 & `"$binPath`" 启动。"
    }

    try {
        $verOutput = & $binPath --version
        if ($LASTEXITCODE -ne 0) { throw "wocli.exe 启动失败 (退出码: $LASTEXITCODE)" }
        Write-Host "版本: $verOutput"
    } catch { throw "安装的程序无法启动: $_" }
    Write-Success "wocli-music $tag 已成功安装至: $binPath"

    Write-Host @"

==> 🎉 安装完成！快速开始：
  • 当前 PowerShell 可直接运行 wocli；其他已打开终端需重启终端应用
  • 启动终端播放器:  wocli
  • 搜索并播放音乐:  wocli play "周杰伦"
  • 查看全部命令:    wocli --help

"@
} finally {
    if (Test-Path $tempDir) {
        Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
    }
}
