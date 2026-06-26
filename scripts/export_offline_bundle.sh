#!/usr/bin/env bash
set -euo pipefail
OUTPUT_DIR="./offline-bundle"
VERSION="${SCT_VERSION:?set SCT_VERSION}"
IMAGE="ghcr.io/synapticfour/sct-daemon:${VERSION}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output-dir) OUTPUT_DIR="$2"; shift 2 ;;
    --version) VERSION="$2"; IMAGE="ghcr.io/synapticfour/sct-daemon:${VERSION}"; shift 2 ;;
    *) echo "Unknown arg: $1"; exit 1 ;;
  esac
done

mkdir -p "${OUTPUT_DIR}"
ARCHIVE="${OUTPUT_DIR}/images-${VERSION}.tar"
docker pull "${IMAGE}"
docker save "${IMAGE}" -o "${ARCHIVE}"
gzip -f "${ARCHIVE}"
cat > "${OUTPUT_DIR}/manifest.txt" <<EOF
sct_version=${VERSION}
image=${IMAGE}
generated_at=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
EOF
( cd "${OUTPUT_DIR}" && shasum -a 256 ./* > checksums.sha256 )
echo "Offline bundle ready: ${OUTPUT_DIR}"
