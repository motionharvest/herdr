#!/usr/bin/env bash
# Install herdr from GitHub Releases.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/motionharvest/herdr/main/install.sh | bash
#
# Options (environment variables):
#   HERDR_VERSION   Pin a release tag, e.g. v0.7.0 (default: latest)
#   INSTALL_DIR     Where to put the binary (default: ~/.local/bin)
#   HERDR_REPO      GitHub repo slug (default: motionharvest/herdr)
set -euo pipefail

REPO="${HERDR_REPO:-motionharvest/herdr}"
INSTALL_DIR="${INSTALL_DIR:-${HOME}/.local/bin}"
VERSION="${HERDR_VERSION:-}"
BIN="herdr"

err() {
  echo "install.sh: $*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || err "missing required command: $1"
}

detect_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "${os}:${arch}" in
    Linux:x86_64 | Linux:amd64) echo "linux-x86_64" ;;
    Linux:aarch64 | Linux:arm64) echo "linux-aarch64" ;;
    Darwin:x86_64) echo "macos-x86_64" ;;
    Darwin:arm64 | Darwin:aarch64) echo "macos-aarch64" ;;
    *) err "unsupported platform: ${os} ${arch}" ;;
  esac
}

asset_name() {
  local target="$1"
  echo "herdr-${target}"
}

normalize_version() {
  local version="$1"
  case "${version}" in
    v*) echo "${version}" ;;
    *) echo "v${version}" ;;
  esac
}

fetch_latest_version() {
  need_cmd curl
  local tag
  tag="$(
    curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
      | sed -n 's/.*"tag_name": "\([^"]*\)".*/\1/p' \
      | head -n1
  )"
  if [[ -z "${tag}" ]]; then
    err "no releases found at https://github.com/${REPO}/releases — publish a release first, or install upstream with HERDR_REPO=ogulcancelik/herdr"
  fi
  echo "${tag}"
}

download_release() {
  local version="$1"
  local target="$2"
  local asset url

  version="$(normalize_version "${version}")"
  asset="$(asset_name "${target}")"
  url="https://github.com/${REPO}/releases/download/${version}/${asset}"

  need_cmd curl

  local tmpdir
  tmpdir="$(mktemp -d)"
  trap 'rm -rf -- "$tmpdir"' RETURN

  echo "Downloading ${url} ..."
  if ! curl -fsSL "${url}" -o "${tmpdir}/${BIN}"; then
    err "download failed from ${url} — check that ${version} includes asset ${asset}"
  fi

  mkdir -p "${INSTALL_DIR}"
  install -m 0755 "${tmpdir}/${BIN}" "${INSTALL_DIR}/${BIN}"
  rm -rf -- "${tmpdir}"
  trap - RETURN
}

main() {
  need_cmd uname

  if [[ -z "${VERSION}" ]]; then
    VERSION="$(fetch_latest_version)"
  else
    VERSION="$(normalize_version "${VERSION}")"
  fi

  local target
  target="$(detect_target)"
  echo "Installing herdr ${VERSION} (${target}) from ${REPO} into ${INSTALL_DIR} ..."

  download_release "${VERSION}" "${target}"

  if [[ ":${PATH}:" != *":${INSTALL_DIR}:"* ]]; then
    echo
    echo "Installed ${INSTALL_DIR}/${BIN}"
    echo "Add this to your shell profile if needed:"
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
  else
    echo "Installed ${INSTALL_DIR}/${BIN}"
  fi

  echo
  echo "Run 'herdr' to get started."
}

main "$@"
