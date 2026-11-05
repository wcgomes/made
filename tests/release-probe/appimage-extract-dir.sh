#!/usr/bin/env bash
# Security probe M2: where does the AppImage runtime extract, and can another
# local user plant the extraction directory? Uses the published v0.1.6
# AppImage, the pinned uruntime, and a second local user `attacker`.
# Prints PASS/FAIL lines; exits non-zero on any FAIL. Removed with the probes.
set -uo pipefail

runtime="${1:?uruntime path}"
work="$(mktemp -d)"
cd "$work"
fail=0
pass() { echo "PASS $*"; }
bad() { echo "FAIL $*"; fail=1; }

gh release download v0.1.6 --repo wcgomes/made --pattern 'made-0.1.6-linux-x64.AppImage' --dir .
old="$PWD/made-0.1.6-linux-x64.AppImage"
chmod 755 "$old"
"$old" --appimage-envs | sed 's/^/old envs: /'

# 1. The shipped runtime extracts to a predictable shared path in /tmp.
rm -rf /tmp/appimage_extracted_*
NO_CLEANUP=1 "$old" --version
predictable="$(ls -d /tmp/appimage_extracted_* 2>/dev/null | head -1)"
echo "shipped extraction dir: ${predictable:-none}"
stat -c '%U %a %n' "$predictable" 2>/dev/null
rm -rf /tmp/appimage_extracted_*

# 2. Another user plants that path; the shipped runtime runs the planted code.
sudo useradd -m attacker 2>/dev/null || true
sudo -u attacker bash -c "mkdir -p '$predictable/usr/bin' && printf '#!/bin/sh\necho PWNED-BY-ATTACKER\n' > '$predictable/AppRun' && cp '$predictable/AppRun' '$predictable/usr/bin/made-app' && chmod -R 755 '$predictable'"
out="$("$old" --version 2>&1)"
echo "shipped runtime with planted dir: $out"
case "$out" in
  *PWNED*) echo "REPRODUCED: the shipped runtime executes another user's planted directory" ;;
  *) echo "NOT REPRODUCED: the shipped runtime did not run the planted directory" ;;
esac

# 3. Rebuild with a per-user, per-version extraction target.
offset="$("$old" --appimage-offset)"
new="$PWD/made-fixed.AppImage"
{ cat "$runtime"; tail -c +"$((offset + 1))" "$old"; } > "$new"
chmod 755 "$new"
envs=$'APPIMAGE_EXTRACT_AND_RUN=1\nAPPIMAGE_TARGET_DIR=${HOME}/.cache/made/appimage-0.1.6'
"$new" --appimage-addenvs "$envs"
"$new" --appimage-envs | sed 's/^/new envs: /'

rm -rf "$HOME/.cache/made"
out="$(NO_CLEANUP=1 "$new" --version 2>&1)"
echo "fixed runtime: $out"
[ "$out" = "Made 0.1.6" ] && pass "fixed runtime starts Made despite the planted /tmp directory" || bad "fixed runtime output: $out"
target="$HOME/.cache/made/appimage-0.1.6"
if [ -d "$target" ]; then
  meta="$(stat -c '%U %a' "$target")"
  [ "$meta" = "$(id -un) 700" ] && pass "extraction target $target is owned by the user with mode 700" || bad "target metadata: $meta"
else
  bad "the expanded per-user target was not used (no $target)"
fi
leftover="$(ls -d /tmp/appimage_extracted_* 2>/dev/null | grep -v "^$predictable$" || true)"
[ -z "$leftover" ] && pass "the fixed runtime creates nothing under /tmp" || bad "fixed runtime used /tmp: $leftover"
rm -rf "$HOME/.cache/made"

# 4. Another user cannot plant the per-user target.
if sudo -u attacker mkdir -p "$target" 2>/dev/null; then
  bad "attacker could create $target"
else
  pass "attacker cannot create the per-user target ($(stat -c '%U %a' "$HOME") home)"
fi

# 5. A user TMPDIR does not move the target back to a shared directory.
out="$(TMPDIR=/tmp NO_CLEANUP=1 "$new" --version 2>&1)"
[ "$out" = "Made 0.1.6" ] && [ -d "$target" ] && pass "TMPDIR=/tmp still extracts to the per-user target" || bad "TMPDIR override: $out"
rm -rf "$HOME/.cache/made"

# 6. The target variable does not leak into Made's environment.
out="$("$new" --appimage-extract-and-run --version 2>&1 | head -1)"
echo "explicit extract-and-run: $out"

exit "$fail"
