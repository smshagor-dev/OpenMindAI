#!/usr/bin/env bash
# OpenMindAI universal Linux bootstrap launcher.
# Version: 3.0.1
# Kept intentionally small -- real logic lives in bootstrap/linux/bootstrap.sh.
# Usage: chmod +x openmindai-setup.sh && ./openmindai-setup.sh
set -u

# Resolve this script's own real directory, even through symlinks and when
# invoked from another working directory (works from any mount point,
# including a path with spaces -- everything below is quoted).
resolve_script_dir() {
  local source="${BASH_SOURCE[0]:-$0}"
  while [ -h "$source" ]; do
    local dir
    dir="$(cd -P "$(dirname "$source")" >/dev/null 2>&1 && pwd)"
    source="$(readlink "$source")"
    case "$source" in
      /*) ;;
      *) source="$dir/$source" ;;
    esac
  done
  cd -P "$(dirname "$source")" >/dev/null 2>&1 && pwd
}

SCRIPT_DIR="$(resolve_script_dir)"
BOOTSTRAP_SH="$SCRIPT_DIR/bootstrap/linux/bootstrap.sh"

if [ ! -f "$BOOTSTRAP_SH" ]; then
  echo ""
  echo "[OpenMindAI Setup] bootstrap/linux/bootstrap.sh was not found next to this file."
  echo "This looks like a standalone copy of openmindai-setup.sh -- fetching the"
  echo "bootstrap script from the official repository..."
  echo ""
  mkdir -p "$SCRIPT_DIR/bootstrap/linux"
  RAW_URL="https://raw.githubusercontent.com/smshagor-dev/OpenMindAI/main/bootstrap/linux/bootstrap.sh"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$RAW_URL" -o "$BOOTSTRAP_SH"
  elif command -v wget >/dev/null 2>&1; then
    wget -q "$RAW_URL" -O "$BOOTSTRAP_SH"
  else
    echo "[OpenMindAI Setup] Neither curl nor wget is available -- cannot fetch the bootstrap script."
    echo "Install curl or wget, or download the full OpenMindAI repository instead:"
    echo "https://github.com/smshagor-dev/OpenMindAI"
    exit 1
  fi
  if [ ! -s "$BOOTSTRAP_SH" ]; then
    echo "[OpenMindAI Setup] Could not download the bootstrap script. Check your internet connection."
    exit 1
  fi
fi

chmod +x "$BOOTSTRAP_SH" 2>/dev/null || true
exec "$BOOTSTRAP_SH" --launcher-root "$SCRIPT_DIR" "$@"
