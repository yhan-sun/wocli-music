#!/usr/bin/env bash
# wocli-music installer
# https://github.com/yhan-sun/wocli-music

set -euo pipefail

REPO="yhan-sun/wocli-music"
DEFAULT_TAG="v0.2.0"
TMPDIR_CLEANUP=""

cleanup() {
    if [ -n "${TMPDIR_CLEANUP:-}" ] && [ -d "$TMPDIR_CLEANUP" ]; then
        rm -rf "$TMPDIR_CLEANUP"
    fi
}
trap cleanup EXIT

# Colors for terminal output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
NC='\033[0m'

info() {
    printf "${BLUE}==>${NC} ${BOLD}%s${NC}\n" "$1"
}

success() {
    printf "${GREEN}==>${NC} ${BOLD}%s${NC}\n" "$1"
}

warn() {
    printf "${YELLOW}警告:${NC} %s\n" "$1"
}

error() {
    printf "${RED}错误:${NC} %s\n" "$1" >&2
    exit 1
}

usage() {
    cat <<EOF
wocli-music 官方安装脚本

用法:
  curl -fsSL https://raw.githubusercontent.com/yhan-sun/wocli-music/main/install.sh | bash
  或: ./install.sh [选项]

选项:
  -d, --dir <DIR>       指定可执行文件安装目录 (默认: ~/.local/bin 或 /usr/local/bin)
  -v, --version <VER>   指定安装版本 (默认: 最新正式发布版)
  -u, --uninstall       卸载 wocli-music
  -h, --help            显示帮助信息

环境变量:
  INSTALL_DIR           覆盖可执行文件安装目录
  VERSION               覆盖安装版本 (例如 v0.2.0)
EOF
}

detect_platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Darwin)
            case "$arch" in
                arm64|aarch64) PLATFORM="macos-arm64" ;;
                x86_64|amd64)  PLATFORM="macos-x86_64" ;;
                *) error "不支持的 macOS 架构: $arch" ;;
            esac
            ;;
        Linux)
            case "$arch" in
                x86_64|amd64)  PLATFORM="linux-x86_64" ;;
                *) error "暂不支持此 Linux 架构: $arch (当前发行版仅提供 linux-x86_64)" ;;
            esac
            ;;
        *)
            error "不支持的操作系统: $os (wocli-music 当前支持 macOS 15+ 与 Linux x86_64)"
            ;;
    esac
}

get_downloader() {
    if command -v curl >/dev/null 2>&1; then
        DOWNLOADER="curl"
    elif command -v wget >/dev/null 2>&1; then
        DOWNLOADER="wget"
    else
        error "未找到 curl 或 wget，请先安装其中之一。"
    fi
}

download_file() {
    local url="$1"
    local dest="$2"
    if [ "$DOWNLOADER" = "curl" ]; then
        curl -fsSL "$url" -o "$dest"
    else
        wget -q "$url" -O "$dest"
    fi
}

fetch_latest_tag() {
    if [ -n "${VERSION:-}" ]; then
        TAG="$VERSION"
        return
    fi

    info "正在查询最新版本..."
    local latest=""
    if [ "$DOWNLOADER" = "curl" ]; then
        latest=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/' || true)
    else
        latest=$(wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/' || true)
    fi

    if [ -n "$latest" ]; then
        TAG="$latest"
    else
        TAG="$DEFAULT_TAG"
        warn "获取最新版本信息失败，将默认安装 $TAG"
    fi
}

verify_sha256() {
    local file="$1"
    local expected="$2"
    local actual=""

    if command -v shasum >/dev/null 2>&1; then
        actual=$(shasum -a 256 "$file" | awk '{print $1}')
    elif command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$file" | awk '{print $1}')
    else
        warn "系统未找到 shasum 或 sha256sum，跳过校验"
        return 0
    fi

    if [ "$actual" != "$expected" ]; then
        error "SHA256 校验失败！\n期望值: $expected\n实际值: $actual"
    fi
}

uninstall() {
    info "正在卸载 wocli-music..."

    if command -v wocli >/dev/null 2>&1; then
        info "正在停止运行中的后台服务..."
        wocli daemon stop 2>/dev/null || true
    fi

    local removed=0
    for target in "$INSTALL_DIR/wocli" "$HOME/.local/bin/wocli" "/usr/local/bin/wocli"; do
        if [ -f "$target" ] || [ -L "$target" ]; then
            rm -f "$target"
            success "已删除: $target"
            removed=1
        fi
    done

    for shared in "$HOME/.local/share/wocli-music" "/usr/local/share/wocli-music"; do
        if [ -d "$shared" ]; then
            rm -rf "$shared"
            success "已删除: $shared"
            removed=1
        fi
    done

    if [ $removed -eq 1 ]; then
        success "wocli-music 已成功卸载。"
        printf "\n注意：你的曲库数据与设置仍保存在 ~/.config/wocli-music 中。\n如需彻底清除，可手动删除该目录。\n"
    else
        warn "未找到已安装的 wocli 二进制文件。"
    fi
    exit 0
}

main() {
    INSTALL_DIR="${INSTALL_DIR:-}"
    VERSION="${VERSION:-}"
    UNINSTALL=0

    while [ $# -gt 0 ]; do
        case "$1" in
            -d|--dir)
                INSTALL_DIR="$2"
                shift 2
                ;;
            -v|--version)
                VERSION="$2"
                shift 2
                ;;
            -u|--uninstall)
                UNINSTALL=1
                shift
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            *)
                error "未知选项: $1 (使用 --help 查看选项)"
                ;;
        esac
    done

    if [ -z "$INSTALL_DIR" ]; then
        if [ "$(id -u)" -eq 0 ]; then
            INSTALL_DIR="/usr/local/bin"
        else
            INSTALL_DIR="$HOME/.local/bin"
        fi
    fi

    if [ $UNINSTALL -eq 1 ]; then
        uninstall
    fi

    detect_platform
    get_downloader
    fetch_latest_tag

    local version_clean="${TAG#v}"
    local filename="wocli-music-${version_clean}-${PLATFORM}.tar.gz"
    local download_url="https://github.com/${REPO}/releases/download/${TAG}/${filename}"
    local sums_url="https://github.com/${REPO}/releases/download/${TAG}/SHA256SUMS"

    TMPDIR_CLEANUP="$(mktemp -d 2>/dev/null || mktemp -d -t 'wocli-install')"
    local tmpdir="$TMPDIR_CLEANUP"

    info "平台: ${PLATFORM}"
    info "目标版本: ${TAG}"
    info "下载地址: ${download_url}"

    download_file "$download_url" "$tmpdir/$filename"
    download_file "$sums_url" "$tmpdir/SHA256SUMS"

    local expected_hash
    expected_hash=$(grep "  ${filename}\$" "$tmpdir/SHA256SUMS" | awk '{print $1}' || true)
    if [ -z "$expected_hash" ]; then
        expected_hash=$(grep "${filename}" "$tmpdir/SHA256SUMS" | awk '{print $1}' || true)
    fi

    if [ -n "$expected_hash" ]; then
        info "正在校验 SHA256 签名..."
        verify_sha256 "$tmpdir/$filename" "$expected_hash"
        success "SHA256 校验通过: $expected_hash"
    else
        warn "未在 SHA256SUMS 中找到该文件的校验值，跳过签名检查"
    fi

    info "正在解压安装包..."
    tar -xzf "$tmpdir/$filename" -C "$tmpdir"
    local extract_dir="$tmpdir/wocli-music-${version_clean}-${PLATFORM}"

    mkdir -p "$INSTALL_DIR"

    if [ "$(uname -s)" = "Darwin" ]; then
        cp -f "$extract_dir/wocli" "$INSTALL_DIR/wocli"
        chmod +x "$INSTALL_DIR/wocli"
        xattr -d com.apple.quarantine "$INSTALL_DIR/wocli" 2>/dev/null || true
    else
        # Linux: keep lib/ adjacent to binary for ALSA rpath
        local share_dir
        if [ "$INSTALL_DIR" = "/usr/local/bin" ] || [ "$INSTALL_DIR" = "/usr/bin" ]; then
            share_dir="/usr/local/share/wocli-music"
        else
            share_dir="$HOME/.local/share/wocli-music"
        fi
        mkdir -p "$share_dir"
        cp -rf "$extract_dir/"* "$share_dir/"
        chmod +x "$share_dir/wocli"

        # Create wrapper in INSTALL_DIR
        cat > "$INSTALL_DIR/wocli" <<WRAPPER
#!/bin/sh
exec "$share_dir/wocli" "\$@"
WRAPPER
        chmod +x "$INSTALL_DIR/wocli"
    fi

    success "wocli-music ${TAG} 已成功安装至: $INSTALL_DIR/wocli"

    # Verify installation
    if [ -x "$INSTALL_DIR/wocli" ]; then
        local ver
        ver=$("$INSTALL_DIR/wocli" --version 2>/dev/null || true)
        if [ -n "$ver" ]; then
            printf "版本信息: %s\n" "$ver"
        fi
    fi

    # Check PATH
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) ;;
        *)
            printf "\n"
            warn "$INSTALL_DIR 不在你的 PATH 环境变量中。"
            printf "请将以下内容添加到你的终端配置文件 (~/.bashrc, ~/.zshrc 等):\n\n"
            printf "  ${BOLD}export PATH=\"%s:\$PATH\"${NC}\n\n" "$INSTALL_DIR"
            printf "然后执行: ${BOLD}source ~/.zshrc${NC} (或重开终端)\n"
            ;;
    esac

    printf "\n"
    success "🎉 安装完成！快速开始："
    printf "  • 启动终端界面: ${BOLD}wocli${NC}\n"
    printf "  • 搜索并播放:   ${BOLD}wocli play '周杰伦'${NC}\n"
    printf "  • 查看帮助:     ${BOLD}wocli --help${NC}\n"
}

main "$@"
