#!/usr/bin/env bash
set -euo pipefail
BUNDLE_DIR="./offline-bundle"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundle-dir) BUNDLE_DIR="$2"; shift 2 ;;
    *) echo "Unknown arg: $1"; exit 1 ;;
  esac
done
[ -d "${BUNDLE_DIR}" ] || { echo "Bundle not found"; exit 1; }
( cd "${BUNDLE_DIR}" && shasum -a 256 -c checksums.sha256 )
ARCHIVE="$(ls "${BUNDLE_DIR}"/images-*.tar.gz | head -n 1)"
gunzip -c "${ARCHIVE}" | docker load
echo "Import complete. cp sct.toml.example sct.toml, set SCT_VERSION, ./install.sh --offline"
