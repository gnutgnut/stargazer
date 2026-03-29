#!/usr/bin/env bash
set -euo pipefail

# Stargazer dev setup — works on Linux, WSL, and macOS.
# For Windows (native), see the note at the bottom or run this in WSL.

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

info()  { echo -e "${GREEN}[+]${NC} $*"; }
warn()  { echo -e "${YELLOW}[!]${NC} $*"; }
fail()  { echo -e "${RED}[x]${NC} $*"; exit 1; }

OS="$(uname -s)"
ARCH="$(uname -m)"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# ── 1. Rust ──────────────────────────────────────────────────────────────────
if command -v rustc &>/dev/null; then
    info "Rust already installed: $(rustc --version)"
else
    info "Installing Rust via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "${HOME}/.cargo/env" 2>/dev/null || true
    info "Rust installed: $(rustc --version)"
fi

# Make sure cargo is on PATH for the rest of this script
export PATH="${HOME}/.cargo/bin:${PATH}"

# ── 2. C linker ─────────────────────────────────────────────────────────────
#
# Rust needs a C linker. We check for one, and if missing, install the
# lightest option that works on the current platform.

has_c_linker() {
    command -v cc &>/dev/null || command -v gcc &>/dev/null || command -v clang &>/dev/null
}

install_linker_linux() {
    if command -v apt-get &>/dev/null; then
        info "Installing gcc via apt..."
        sudo apt-get update -qq && sudo apt-get install -y -qq build-essential
    elif command -v dnf &>/dev/null; then
        info "Installing gcc via dnf..."
        sudo dnf install -y gcc
    elif command -v pacman &>/dev/null; then
        info "Installing gcc via pacman..."
        sudo pacman -S --noconfirm base-devel
    elif command -v apk &>/dev/null; then
        info "Installing gcc via apk..."
        sudo apk add build-base
    elif command -v zypper &>/dev/null; then
        info "Installing gcc via zypper..."
        sudo zypper install -y gcc
    else
        return 1
    fi
}

install_zig_as_linker() {
    # Zig is a single static binary that includes a C compiler.
    # Works without root, no package manager needed.
    local ZIG_VER="0.13.0"
    local ZIG_DIR="/tmp/zig-${ZIG_VER}"

    if [[ -x "${HOME}/.local/bin/zig" ]]; then
        info "Zig already available at ~/.local/bin/zig"
    else
        info "No system C compiler and no sudo — downloading zig as a fallback linker..."
        mkdir -p "${HOME}/.local/bin"

        local ZIG_TAR=""
        case "${OS}-${ARCH}" in
            Linux-x86_64)   ZIG_TAR="zig-linux-x86_64-${ZIG_VER}.tar.xz" ;;
            Linux-aarch64)  ZIG_TAR="zig-linux-aarch64-${ZIG_VER}.tar.xz" ;;
            Darwin-x86_64)  ZIG_TAR="zig-macos-x86_64-${ZIG_VER}.tar.xz" ;;
            Darwin-arm64)   ZIG_TAR="zig-macos-aarch64-${ZIG_VER}.tar.xz" ;;
            *) fail "No zig binary available for ${OS}-${ARCH}" ;;
        esac

        local URL="https://ziglang.org/download/${ZIG_VER}/${ZIG_TAR}"
        info "Downloading ${URL}..."
        curl -sL "${URL}" -o "/tmp/${ZIG_TAR}"
        mkdir -p "${ZIG_DIR}"
        tar xf "/tmp/${ZIG_TAR}" -C "${ZIG_DIR}" --strip-components=1
        ln -sf "${ZIG_DIR}/zig" "${HOME}/.local/bin/zig"
        rm -f "/tmp/${ZIG_TAR}"
        info "Zig installed to ~/.local/bin/zig"
    fi

    # Write the wrapper script and cargo config
    local ZIGCC="${SCRIPT_DIR}/zigcc.sh"
    cat > "${ZIGCC}" <<'ZIGEOF'
#!/bin/sh
ZIG="${HOME}/.local/bin/zig"
args=""
for arg in "$@"; do
    case "$arg" in
        --target=*-unknown-linux-gnu)
            triple="$(echo "$arg" | sed 's/--target=//;s/-unknown-linux-gnu/-linux-gnu/')"
            args="$args --target=$triple"
            ;;
        *)
            args="$args $arg"
            ;;
    esac
done
exec "$ZIG" cc $args
ZIGEOF
    chmod +x "${ZIGCC}"

    mkdir -p "${SCRIPT_DIR}/.cargo"
    cat > "${SCRIPT_DIR}/.cargo/config.toml" <<CFGEOF
[target.x86_64-unknown-linux-gnu]
linker = "${ZIGCC}"

[target.aarch64-unknown-linux-gnu]
linker = "${ZIGCC}"
CFGEOF
    info "Configured cargo to use zig cc as linker"
}

if has_c_linker; then
    info "C linker found: $(command -v cc || command -v gcc || command -v clang)"
    # Clean up zig workaround if a real compiler is now available
    rm -f "${SCRIPT_DIR}/.cargo/config.toml" 2>/dev/null || true
elif [[ "$OS" == "Darwin" ]]; then
    info "Installing Xcode Command Line Tools (provides clang)..."
    xcode-select --install 2>/dev/null || true
    warn "If a dialog appeared, click Install and re-run this script."
    warn "If already installed, ignore the error above."
    if ! has_c_linker; then
        fail "Still no C compiler after xcode-select. Install Xcode CLT manually."
    fi
elif [[ "$OS" == "Linux" ]]; then
    # Try package manager first (needs sudo), fall back to zig (no sudo)
    if sudo -n true 2>/dev/null; then
        install_linker_linux || install_zig_as_linker
    else
        warn "No C compiler found and no passwordless sudo."
        install_zig_as_linker
    fi
else
    fail "Unsupported OS: ${OS}. On Windows, use WSL or install Visual Studio Build Tools."
fi

# ── 3. Build ─────────────────────────────────────────────────────────────────
info "Building stargazer (release)..."
cd "${SCRIPT_DIR}"
export PATH="${HOME}/.local/bin:${PATH}"
CC="${SCRIPT_DIR}/zigcc.sh" cargo build --release 2>&1

info "Done! Run with:"
echo "  ./target/release/stargazer"
echo ""
echo "Controls: ESC or Q to quit"

# ── Windows (native, no WSL) ────────────────────────────────────────────────
# If you're on Windows without WSL:
#   1. Install Rust: https://rustup.rs
#   2. Install Visual Studio Build Tools (C++ workload)
#      https://visualstudio.microsoft.com/visual-cpp-build-tools/
#   3. Open "x64 Native Tools Command Prompt" and run:
#        cargo build --release
#        target\release\stargazer.exe
