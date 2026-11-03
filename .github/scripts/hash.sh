#!/usr/bin/env bash
# Shared sha256 helper for companion staging. Lowercase hex, 64 characters.
sha256_file() {
  local file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -- "$file" | awk '{print $1}'
    return
  fi
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -- "$file" | awk '{print $1}'
    return
  fi
  local py="python3"
  if ! command -v python3 >/dev/null 2>&1; then
    py="python"
  fi
  "$py" -c 'import hashlib, pathlib, sys; print(hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest())' "$file"
}
