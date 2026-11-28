# Proposal: SSH Grok home-directory detection fallback

## Intent
Grok's installer exposes `~/.grok/bin` only through interactive shell startup files, so a PATH-only lookup run over a non-interactive SSH command misses a correctly installed grok and `Grok SSH` is never offered. Detection must also find grok in the user's install locations.

## Scope
Modifies SSH-01 (detection wording and the `Grok SSH` scenario), SSH-09 (the PATH-only statement for grok) and SSH-10 (detection rule, one consistency guarantee, the detection scenario and two new scenarios) in `specs/ssh-remote-workspaces.md`. The change is already implemented; this records it as the contract.

## Non-goals
- No change to Claude Code detection, which stays PATH-only under SSH-11, and no change to OpenCode detection.
- No change to the `Grok SSH` launch arguments, the consented install flow, hook tiers or any remote write rule.
- No version check, no spawning grok and no interpreter requirement.

## Approach
Detection keeps the bounded metadata-only PATH lookup first and falls back, only when it finds nothing, to three fixed install locations under the remote user's home in a fixed order. One detected binary is shared by the launch bar, the launch, the consented install and the attached session's capability report.

## Decision index
- **PATH first, then fixed install locations:** a PATH hit always wins and keeps the user's intent; the fallback only fills the gap the non-interactive shell leaves.
- **Order `.grok/bin`, `.local/bin`, `.cargo/bin`:** official install location first, then common user bin directories.
- **One detected binary for every consumer:** a launch bar offer that a later step reports as missing is a user-visible contradiction, so the guarantee is normative.
- **Claude Code stays PATH-only:** no evidence it needs the fallback; widening it is a separate change.
- **SSH-09 included:** its statement that grok is detected only on PATH would contradict SSH-10.
