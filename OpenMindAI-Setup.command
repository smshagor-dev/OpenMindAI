#!/usr/bin/env bash
# OpenMindAI universal macOS bootstrap launcher.
# Version: 3.0.0
# Double-click in Finder, or run: chmod +x OpenMindAI-Setup.command && ./OpenMindAI-Setup.command
# Kept intentionally small -- real logic lives in bootstrap/macos/bootstrap.sh.
set -u

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
BOOTSTRAP_SH="$SCRIPT_DIR/bootstrap/macos/bootstrap.sh"

# Finder double-click launches this in a fresh Terminal window that closes
# immediately on exit -- pause on failure so the user can actually read it.
pause_on_exit() {
  local code=$?
  if [ $code -ne 0 ]; then
    echo ""
    echo "Press Enter to close this window..."
    read -r _ || true
  fi
  exit $code
}
trap pause_on_exit EXIT

if [ ! -f "$BOOTSTRAP_SH" ]; then
  echo ""
  echo "[OpenMindAI Setup] bootstrap/macos/bootstrap.sh was not found next to this file."
  echo "This looks like a standalone copy of OpenMindAI-Setup.command -- fetching the"
  echo "bootstrap script from the official repository..."
  echo ""
  mkdir -p "$SCRIPT_DIR/bootstrap/macos"
  RAW_URL="https://raw.githubusercontent.com/smshagor-dev/OpenMindAI/main/bootstrap/macos/bootstrap.sh"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$RAW_URL" -o "$BOOTSTRAP_SH"
  else
    echo "[OpenMindAI Setup] curl is not available -- cannot fetch the bootstrap script."
    echo "Download the full OpenMindAI repository instead: https://github.com/smshagor-dev/OpenMindAI"
    exit 1
  fi
  if [ ! -s "$BOOTSTRAP_SH" ]; then
    echo "[OpenMindAI Setup] Could not download the bootstrap script. Check your internet connection."
    exit 1
  fi
fi

chmod +x "$BOOTSTRAP_SH" 2>/dev/null || true
"$BOOTSTRAP_SH" --launcher-root "$SCRIPT_DIR" "$@"
