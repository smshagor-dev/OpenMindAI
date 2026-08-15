#!/usr/bin/env bash
# OpenMindAI macOS bootstrap. Invoked by OpenMindAI-Setup.command.
#
# Lifecycle: locate/clone source -> resolve a prebuilt release for this
# architecture (preferred) -> fall back to building from source (developer
# mode only) -> launch OpenMindAI.
#
# Safe to run repeatedly: an already-installed OpenMindAI is detected and
# launched directly without re-cloning, re-installing dependencies, or
# rebuilding. Never destroys local modifications in an existing source
# checkout (fetch + fast-forward only).
set -u

LAUNCHER_ROOT=""
DEVELOPER_MODE=0
NO_LAUNCH=0
while [ $# -gt 0 ]; do
  case "$1" in
    --launcher-root) LAUNCHER_ROOT="$2"; shift 2 ;;
    --developer-mode) DEVELOPER_MODE=1; shift ;;
    --no-launch) NO_LAUNCH=1; shift ;;
    *) shift ;;
  esac
done
SCRIPT_DIR="$(cd -P "$(dirname "${BASH_SOURCE[0]:-$0}")" >/dev/null 2>&1 && pwd)"
if [ -z "$LAUNCHER_ROOT" ]; then
  LAUNCHER_ROOT="$(cd -P "$SCRIPT_DIR/../.." >/dev/null 2>&1 && pwd)"
fi

step() { echo "==> $*"; }
info() { echo "    $*"; }
warn() { echo "    $*" >&2; }
err()  { echo "[OpenMindAI Setup] $*" >&2; }

# ---------------------------------------------------------------------
# Repository config -- single source of truth is bootstrap/config/repo.conf.
# Embedded defaults exist only for the standalone-download case (see
# OpenMindAI-Setup.command); they must always match repo.conf.
# ---------------------------------------------------------------------
REPO_URL="https://github.com/smshagor-dev/OpenMindAI.git"
REPO_BRANCH="main"
REPO_OWNER="smshagor-dev"
REPO_NAME="OpenMindAI"
REPO_CONFIG="$SCRIPT_DIR/../config/repo.conf"
if [ -f "$REPO_CONFIG" ]; then
  # shellcheck disable=SC1090
  . "$REPO_CONFIG"
  REPO_URL="${OPENMINDAI_REPO_URL:-$REPO_URL}"
  REPO_BRANCH="${OPENMINDAI_REPO_BRANCH:-$REPO_BRANCH}"
  REPO_OWNER="${OPENMINDAI_REPO_OWNER:-$REPO_OWNER}"
  REPO_NAME="${OPENMINDAI_REPO_NAME:-$REPO_NAME}"
fi

have_internet() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsS -o /dev/null -m 3 https://github.com >/dev/null 2>&1 && return 0
  fi
  timeout 3 bash -c 'cat < /dev/null > /dev/tcp/github.com/443' >/dev/null 2>&1
}

is_valid_source() {
  local path="$1"
  [ -f "$path/package.json" ] && [ -d "$path/src" ] && [ -d "$path/src-tauri" ] && [ -f "$path/README.md" ]
}

resolve_source_root() {
  if is_valid_source "$LAUNCHER_ROOT"; then
    echo "$LAUNCHER_ROOT"
  else
    echo "$LAUNCHER_ROOT/OpenMindAI"
  fi
}

# ---------------------------------------------------------------------
# Git detection. Prefers Apple's own Command Line Tools (the standard,
# Apple-supported way to get `git` on macOS) over Homebrew. Never installs
# Homebrew itself without the user's explicit say-so.
# ---------------------------------------------------------------------
install_git() {
  step "Git is required but was not found."
  if command -v brew >/dev/null 2>&1; then
    info "Homebrew is already installed -- using it to install git."
    brew install git && return 0
  fi
  info "Requesting Xcode Command Line Tools (includes git) -- macOS will show its own install dialog."
  xcode-select --install 2>/dev/null
  err "Finish the Xcode Command Line Tools install in the dialog that just opened, then run setup again."
  info "(Not installing Homebrew automatically -- if you'd rather use it, install it yourself from https://brew.sh/ then run setup again.)"
  return 1
}

# ---------------------------------------------------------------------
# Safe source sync
# ---------------------------------------------------------------------
sync_source() {
  local source_root="$1"
  if [ ! -d "$source_root/.git" ]; then
    if [ -e "$source_root" ]; then
      if is_valid_source "$source_root"; then
        info "Existing OpenMindAI source found at $source_root (not a git checkout) -- using as-is."
        return 0
      fi
      err "$source_root exists but is not a valid OpenMindAI checkout. Remove it or choose a different location."
      return 1
    fi
    step "Cloning OpenMindAI source..."
    info "$REPO_URL -> $source_root"
    if ! git clone --branch "$REPO_BRANCH" "$REPO_URL" "$source_root"; then
      err "git clone failed."
      return 1
    fi
    if ! is_valid_source "$source_root"; then
      err "Cloned repository does not look like OpenMindAI (missing package.json/src/src-tauri/README.md) -- refusing to continue."
      return 1
    fi
    info "Source cloned."
    return 0
  fi

  if ! have_internet; then
    info "Offline -- skipping source update, using existing checkout."
    return 0
  fi

  (
    cd "$source_root" || exit 1
    if [ -n "$(git status --porcelain 2>/dev/null)" ]; then
      warn "Existing source checkout has local changes -- skipping automatic update."
      info "(Not touching it automatically avoids discarding your changes. Run 'git pull' yourself if you want the latest source.)"
      exit 0
    fi
    step "Updating OpenMindAI source..."
    if ! git fetch origin "$REPO_BRANCH" --quiet; then
      warn "git fetch failed; using existing checkout."
      exit 0
    fi
    if ! git merge --ff-only "origin/$REPO_BRANCH" --quiet; then
      warn "Fast-forward update not possible (local history has diverged); using existing checkout."
      exit 0
    fi
    info "Source up to date."
  )
}

# ---------------------------------------------------------------------
# Prebuilt release resolution (preferred path for normal users)
# ---------------------------------------------------------------------
mac_arch_tag() {
  case "$(uname -m)" in
    arm64) echo "aarch64" ;;
    x86_64) echo "x64" ;;
    *) echo "" ;;
  esac
}

resolve_prebuilt_release() {
  have_internet || return 1
  command -v curl >/dev/null 2>&1 || return 1
  local arch_tag
  arch_tag="$(mac_arch_tag)"
  if [ -z "$arch_tag" ]; then
    info "Unrecognized Mac architecture ($(uname -m)) -- falling back to source build."
    return 1
  fi
  local api="https://api.github.com/repos/$REPO_OWNER/$REPO_NAME/releases/latest"
  local response
  response="$(curl -fsS -H "User-Agent: OpenMindAI-Bootstrap" "$api" 2>/dev/null)" || { info "No published GitHub release found yet -- falling back to source build."; return 1; }
  local url
  url="$(printf '%s' "$response" | grep -o '"browser_download_url":[[:space:]]*"[^"]*"' | grep -i '\.dmg"' | grep -i "$arch_tag" | head -n1 | sed -E 's/.*"([^"]+)"$/\1/')"
  if [ -z "$url" ]; then
    info "Latest release has no macOS $arch_tag asset yet -- falling back to source build."
    return 1
  fi
  echo "$url"
}

# ---------------------------------------------------------------------
# Source build fallback
# ---------------------------------------------------------------------
find_cargo_target_dir() {
  local source_root="$1"
  (cd "$source_root/src-tauri" && cargo metadata --no-deps --format-version 1 2>/dev/null) \
    | grep -o '"target_directory":"[^"]*"' | head -n1 | sed -E 's/.*"target_directory":"([^"]*)"/\1/'
}

find_existing_build() {
  local source_root="$1"
  local target_dir
  target_dir="$(find_cargo_target_dir "$source_root")"
  [ -n "$target_dir" ] || return 1
  local app_bundle="$target_dir/release/bundle/macos/OpenMindAI.app"
  if [ -d "$app_bundle" ]; then echo "$app_bundle"; return 0; fi
  local exe="$target_dir/release/open-mind-ai"
  [ -f "$exe" ] && { echo "$exe"; return 0; }
  return 1
}

build_from_source() {
  local source_root="$1"
  step "No prebuilt release available -- building OpenMindAI from source."
  info "This only happens once per machine (or when the source changes)."

  command -v node >/dev/null 2>&1 || { err "Node.js is required to build from source. Install it (e.g. via https://nodejs.org/ or Homebrew) and run setup again."; return 1; }
  command -v npm  >/dev/null 2>&1 || { err "npm is required to build from source (usually installed with Node.js)."; return 1; }
  command -v cargo >/dev/null 2>&1 || { err "Rust/Cargo is required to build from source. Install it from https://rustup.rs/ and run setup again."; return 1; }
  xcode-select -p >/dev/null 2>&1 || { err "Xcode Command Line Tools are required to build from source. Run 'xcode-select --install' and run setup again."; return 1; }

  (
    cd "$source_root" || exit 1
    if [ ! -d node_modules ]; then
      step "Installing frontend dependencies (first run only)..."
      npm install || exit 1
    fi
    step "Building OpenMindAI (this can take several minutes)..."
    npm run tauri -- build --no-bundle || exit 1
  ) || return 1

  find_existing_build "$source_root"
}

# ---------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------
main() {
  step "OpenMindAI Setup"
  info "Launcher: $LAUNCHER_ROOT"
  info "Architecture: $(uname -m) ($([ "$(uname -m)" = "arm64" ] && echo "Apple Silicon" || echo "Intel"))"

  local online=0
  have_internet && online=1
  if [ "$online" = 1 ]; then info "Internet: available"; else info "Internet: unavailable -- using what's already installed"; fi

  local source_root
  source_root="$(resolve_source_root)"

  if ! is_valid_source "$source_root"; then
    if [ "$online" != 1 ]; then
      err "OpenMindAI isn't installed yet and no internet connection is available to set it up. Connect to the internet and run setup again."
      return 1
    fi
    command -v git >/dev/null 2>&1 || install_git || return 1
    sync_source "$source_root" || return 1
  else
    info "Existing OpenMindAI source found at $source_root"
    if [ -d "$source_root/.git" ] && command -v git >/dev/null 2>&1 && [ "$online" = 1 ]; then
      sync_source "$source_root"
    fi
  fi

  local app=""
  if [ "$DEVELOPER_MODE" != 1 ] && app="$(find_existing_build "$source_root")"; then
    info "Using existing OpenMindAI build."
  elif [ "$online" = 1 ]; then
    local asset_url
    if asset_url="$(resolve_prebuilt_release)"; then
      step "Downloading OpenMindAI ($asset_url)..."
      local install_dir="$LAUNCHER_ROOT/release-download"
      mkdir -p "$install_dir"
      local dest="$install_dir/$(basename "$asset_url")"
      curl -fsSL "$asset_url" -o "$dest"
      info "Downloaded to $dest -- open it to install (Gatekeeper will warn on first open unless the app is signed/notarized; see docs/RELEASE.md in the source repo)."
      app="$dest"
    else
      app="$(build_from_source "$source_root")" || return 1
    fi
  else
    err "No installed OpenMindAI build found, and no internet connection is available to install one."
    return 1
  fi

  if [ -n "$app" ] && [ "$NO_LAUNCH" != 1 ]; then
    step "Starting OpenMindAI..."
    if [[ "$app" == *.app ]]; then
      open "$app"
    elif [[ "$app" == *.dmg ]]; then
      open "$app"
    else
      nohup "$app" >/dev/null 2>&1 &
      disown 2>/dev/null || true
    fi
  fi

  echo ""
  echo "OpenMindAI is ready."
  return 0
}

if ! main; then
  err "Setup did not complete successfully."
  exit 1
fi
exit 0
