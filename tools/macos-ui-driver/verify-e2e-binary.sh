#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
BIN="$SCRIPT_DIR/bin/macos-ui-driver"
PROVENANCE="$SCRIPT_DIR/bin/PROVENANCE.md"

source_fingerprint="$(cd "$SCRIPT_DIR" && find Package.swift Sources -type f -print0 | sort -z | xargs -0 shasum -a 256 | shasum -a 256 | awk '{print $1}')"
binary_sha256="$(shasum -a 256 "$BIN" | awk '{print $1}')"
recorded_binary_sha256="$(sed -n 's/^- Binary SHA-256: `\([^`]*\)`.*/\1/p' "$PROVENANCE")"
recorded_source_fingerprint="$(sed -n 's/^- Source fingerprint: `\([^`]*\)`.*/\1/p' "$PROVENANCE")"

if [[ -z "$recorded_binary_sha256" || -z "$recorded_source_fingerprint" ]]; then
  echo "UNKNOWN_BASELINE_PROVENANCE"
  printf 'binary_sha256=%s\n' "$binary_sha256"
  exit 0
fi

if [[ "$source_fingerprint" != "$recorded_source_fingerprint" ]]; then
  echo "STALE_SOURCE"
  printf 'recorded_source_fingerprint=%s\ncurrent_source_fingerprint=%s\n' \
    "$recorded_source_fingerprint" "$source_fingerprint"
  exit 1
fi

if [[ "$binary_sha256" != "$recorded_binary_sha256" ]]; then
  echo "BINARY_MISMATCH"
  printf 'recorded_binary_sha256=%s\ncurrent_binary_sha256=%s\n' \
    "$recorded_binary_sha256" "$binary_sha256"
  exit 1
fi

echo "SYNCED"
printf 'binary_sha256=%s\nsource_fingerprint=%s\n' "$binary_sha256" "$source_fingerprint"
