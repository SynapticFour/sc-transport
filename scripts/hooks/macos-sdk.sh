#!/usr/bin/env bash
# Keep local clang/cargo builds on the macOS SDK that matches this OS.
#
# macOS 27 SDK text stubs name the architecture arm64e.x1. Xcode 26.6 ships
# ld-1267, whose tapi rejects that token, so every Rust build script fails
# before clippy or tests run. clang invokes that Xcode linker even when
# xcrun selects the macOS 27 SDK. The Command Line Tools linker (ld-27037)
# accepts the same unfiltered SDK.
#
# If the default link works, leave the environment alone. If it fails on
# arm64e.x1 and the Command Line Tools linker can link the macOS 27 SDK,
# pass -fuse-ld to that linker and do not pin an older SDK. Only when that
# linker is missing or also fails, select the newest older SDK that links.

clt_ld_path() {
  local p
  for p in /Library/Developer/CommandLineTools/usr/bin/ld; do
    if [[ -x "${p}" ]]; then
      printf '%s\n' "${p}"
      return 0
    fi
  done
  return 1
}

# link_probe OUT [cc args...] — writes cc stderr to OUT.err
link_probe() {
  local out="$1"
  shift
  printf 'int main(void){return 0;}\n' | cc "$@" -x c - -o "${out}" >/dev/null 2>"${out}.err"
}

use_older_linkable_sdk() {
  local tmp="$1"
  local active="$2"
  local default_sdk resolved_default best best_ver parent cand resolved ver seen tested newer
  default_sdk="${active}"
  resolved_default=""
  if [[ -d "${default_sdk}" ]]; then
    resolved_default="$(cd "${default_sdk}" && pwd -P)"
  fi
  best=""
  best_ver=""
  : >"${tmp}/parents"
  if [[ -n "${default_sdk}" && -d "${default_sdk}" ]]; then
    dirname "${default_sdk}" >>"${tmp}/parents"
  fi
  printf '%s\n' \
    /Library/Developer/CommandLineTools/SDKs \
    /Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs \
    >>"${tmp}/parents"

  seen=" "
  tested=" "
  while IFS= read -r parent || [[ -n "${parent}" ]]; do
    [[ -d "${parent}" ]] || continue
    case "${seen}" in
      *" ${parent} "*) continue ;;
    esac
    seen="${seen}${parent} "
    for cand in "${parent}"/MacOSX*.sdk; do
      [[ -d "${cand}" ]] || continue
      resolved="$(cd "${cand}" && pwd -P)"
      if [[ -n "${resolved_default}" && "${resolved}" == "${resolved_default}" ]]; then
        continue
      fi
      if [[ -f "${cand}/usr/lib/libSystem.B.tbd" ]] && grep -q 'arm64e.x1' "${cand}/usr/lib/libSystem.B.tbd"; then
        continue
      fi
      case "${tested}" in
        *" ${resolved} "*) continue ;;
      esac
      tested="${tested}${resolved} "
      if ! link_probe "${tmp}/old" -isysroot "${cand}"; then
        continue
      fi
      ver="$(basename "${cand}" | sed -n 's/^MacOSX\([0-9][0-9.]*\).*/\1/p')"
      if [[ -z "${ver}" ]]; then
        ver="0"
      fi
      newer=0
      if [[ -z "${best}" ]]; then
        newer=1
      elif awk -v a="${ver}" -v b="${best_ver}" 'BEGIN {
        n = split(a, aa, ".")
        m = split(b, bb, ".")
        limit = n
        if (m > limit) limit = m
        for (i = 1; i <= limit; i++) {
          x = aa[i] + 0
          y = bb[i] + 0
          if (x > y) exit 0
          if (x < y) exit 1
        }
        exit 1
      }'; then
        newer=1
      fi
      if [[ "${newer}" -eq 1 ]]; then
        best="${cand}"
        best_ver="${ver}"
      fi
    done
  done <"${tmp}/parents"

  if [[ -z "${best}" ]]; then
    echo "pre-commit: ld cannot read the macOS 27 SDK (arm64e.x1) and no older SDK linked." >&2
    return 1
  fi
  export SDKROOT="${best}"
  echo "pre-commit: Command Line Tools ld is unavailable; using ${SDKROOT}" >&2
}

use_linkable_macos_sdk() {
  if [[ "$(uname -s)" != "Darwin" ]]; then
    return 0
  fi

  local tmp ldpath active
  tmp="$(mktemp -d)"

  # Honour an explicit older SDK. An explicit SDK that still names
  # arm64e.x1 is not linkable with Xcode ld, so keep going.
  if [[ -n "${SDKROOT:-}" && -f "${SDKROOT}/usr/lib/libSystem.B.tbd" ]] && ! grep -q 'arm64e.x1' "${SDKROOT}/usr/lib/libSystem.B.tbd"; then
    rm -rf "${tmp}"
    return 0
  fi

  if link_probe "${tmp}/default"; then
    rm -rf "${tmp}"
    return 0
  fi
  if ! grep -q 'arm64e.x1' "${tmp}/default.err"; then
    rm -rf "${tmp}"
    return 0
  fi

  ldpath="$(clt_ld_path || true)"
  if [[ -n "${ldpath}" ]] && link_probe "${tmp}/clt" -fuse-ld="${ldpath}"; then
    case "${LDFLAGS:-}" in
      *-fuse-ld=*) ;;
      *) export LDFLAGS="${LDFLAGS:+${LDFLAGS} }-fuse-ld=${ldpath}" ;;
    esac
    case "${RUSTFLAGS:-}" in
      *-fuse-ld=*) ;;
      *) export RUSTFLAGS="${RUSTFLAGS:+${RUSTFLAGS} }-C link-arg=-fuse-ld=${ldpath}" ;;
    esac
    # Pin the resolved macOS 27 SDK. Without SDKROOT, rustc records an
    # older SDK (26.5 on this machine) even when xcrun's default is 27.
    local sdk27=""
    sdk27="$(xcrun --show-sdk-path 2>/dev/null || true)"
    if [[ -n "${sdk27}" && -d "${sdk27}" ]]; then
      sdk27="$(cd "${sdk27}" && pwd -P)"
    fi
    if [[ -n "${sdk27}" && -f "${sdk27}/usr/lib/libSystem.B.tbd" ]] && grep -q 'arm64e.x1' "${sdk27}/usr/lib/libSystem.B.tbd"; then
      export SDKROOT="${sdk27}"
    fi
    rm -rf "${tmp}"
    echo "pre-commit: linking the macOS 27 SDK with ${ldpath}" >&2
    return 0
  fi

  active="${SDKROOT:-}"
  if [[ -z "${active}" ]]; then
    active="$(xcrun --show-sdk-path 2>/dev/null || true)"
  fi
  local status=0
  use_older_linkable_sdk "${tmp}" "${active}" || status=$?
  rm -rf "${tmp}"
  return "${status}"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  set -euo pipefail
  use_linkable_macos_sdk
  printf 'SDKROOT=%s\n' "${SDKROOT-<unset>}"
  printf 'RUSTFLAGS=%s\n' "${RUSTFLAGS-<unset>}"
  printf 'LDFLAGS=%s\n' "${LDFLAGS-<unset>}"
fi
