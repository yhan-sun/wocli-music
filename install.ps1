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
$DefaultTag = "v0.3.0"

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

if ($Help) {
    Write-Host @"
wocli-music 官方 Windows 安装脚本

用法:
  irm https://raw.githubusercontent.com/yhan-sun/wocli-music/main/install.ps1 | iex
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

    # 从用户 PATH 移除
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($userPath) {
        $paths = $userPath -split ';' | Where-Object { $_ -and $_.TrimEnd('\') -ne $InstallDir.TrimEnd('\') }
        $newUserPath = $paths -join ';'
        [Environment]::SetEnvironmentVariable("Path", $newUserPath, "User")
        Write-Success "已从用户 PATH 环境变量中移除"
    }

    Write-Success "wocli-music 已成功卸载。"
    Write-Host "`n注意：你的曲库数据与设置保存在 $env:APPDATA\wocli-music 中，如需彻底清理可手动删除该目录。`n"
    exit 0
}

# 平台与架构检测
$arch = $env:PROCESSOR_ARCHITECTURE
if ($arch -ne "AMD64" -and $arch -ne "ARM64") {
    Write-Err "当前 Windows 架构不支持: $arch (仅支持 x86_64 / ARM64 Windows 10/11)"
}
$platform = "windows-x86_64"

# 获取版本号
$tag = $Version
if (-not $tag) {
    Write-Info "正在获取最新版本信息..."
    try {
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -UseBasicParsing -Headers @{ "User-Agent" = "wocli-installer" }
        $tag = $release.tag_name
    } catch {
        $tag = $DefaultTag
        Write-Warn "获取最新版本信息失败，将安装默认版本 $tag"
    }
}

$cleanVer = $tag.TrimStart('v')
$archiveName = "wocli-music-$cleanVer-$platform.zip"
$downloadUrl = "https://github.com/$Repo/releases/download/$tag/$archiveName"
$sumsUrl = "https://github.com/$Repo/releases/download/$tag/SHA256SUMS"

$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) "wocli-install-$([System.Guid]::NewGuid().ToString().Substring(0,8))"
New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

try {
    Write-Info "目标版本: $tag"
    Write-Info "下载地址: $downloadUrl"

    $zipPath = Join-Path $tempDir $archiveName
    $sumsPath = Join-Path $tempDir "SHA256SUMS"

    if (Get-Command curl.exe -ErrorAction SilentlyContinue) {
        curl.exe -fL "$downloadUrl" -o "$zipPath"
        curl.exe -fL "$sumsUrl" -o "$sumsPath"
    } else {
        Invoke-WebRequest -Uri $downloadUrl -OutFile $zipPath -UseBasicParsing
        Invoke-WebRequest -Uri $sumsUrl -OutFile $sumsPath -UseBasicParsing
    }

    Write-Info "正在校验 SHA256 签名..."
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

    if ($expected -and $hash -ne $expected) {
        Write-Err "SHA256 校验失败！`n期望: $expected`n实际: $hash"
    } elseif ($expected) {
        Write-Success "SHA256 校验通过: $hash"
    }

    Write-Info "正在解压安装..."
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    Expand-Archive -Path $zipPath -DestinationPath $tempDir -Force

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

    # 配置环境变量 PATH
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if (-not $userPath) { $userPath = "" }
    $pathParts = $userPath -split ';' | Where-Object { $_ }

    $normInstall = $InstallDir.TrimEnd('\')
    $alreadyInPath = $false
    foreach ($p in $pathParts) {
        if ($p.TrimEnd('\') -eq $normInstall) {
            $alreadyInPath = $true
            break
        }
    }

    if (-not $alreadyInPath) {
        $newUserPath = if ($userPath) { "$userPath;$InstallDir" } else { $InstallDir }
        [Environment]::SetEnvironmentVariable("Path", $newUserPath, "User")
        $env:Path = "$env:Path;$InstallDir"
        Write-Success "已将 $InstallDir 添加到用户 PATH 环境变量"
    }

    Write-Success "wocli-music $tag 已成功安装至: $binPath"

    try {
        $verOutput = & $binPath --version
        Write-Host "版本: $verOutput"
    } catch {}

    Write-Host @"

==> 🎉 安装完成！快速开始：
  • 打开新的终端窗口 (PowerShell / Windows Terminal)
  • 启动终端播放器:  wocli
  • 搜索并播放音乐:  wocli play "周杰伦"
  • 查看全部命令:    wocli --help

"@
} finally {
    if (Test-Path $tempDir) {
        Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
    }
}
