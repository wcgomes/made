# Native Grok hook acceptance (grok-native-hook, task 7.3)

Contract: `specs/changes/grok-native-hook/`. This file records, per platform, what was executed and what was
not. A combination that was not executed is labelled `UNVERIFIED` with its reason (SSH-09 evidence rules);
nothing here certifies parity for an unexecuted combination.

## Environment

| Item | Value |
|---|---|
| grok | 1.0.70 (stable), installed from `https://x.ai/cli/install.sh` / `install.ps1` on runners; local Linux install |
| Sign-in | None. No signed-in grok exists on the development machine or on hosted runners (`Not signed in`), and no `XAI_API_KEY` was available |
| Hook | `made-grok-status` (`.exe` on Windows), release build, staged by `.github/scripts/stage-grok-plugin.sh` |
| Evidence classes | `CI-real-grok-install` (real grok installs the packaged layout on a runner), `CI-test` (automated test on a runner), `local-test`, `UNVERIFIED` |

## Phase-0 facts (real grok)

| Fact | Result | Evidence |
|---|---|---|
| `grok plugin install <dir> --trust` copies instead of linking | Copies on Linux, macOS arm64, macOS x64, Windows | probe run 41971944131; no symlinks, registry records only the source path |
| Executable mode preserved (700, 755, 644) | Preserved on Linux, macOS arm64, macOS x64 | same run |
| Windows resolves `bin/made-grok-status` to `.exe` | Not established; generated Windows `hooks.json` with `.exe` selected | grok 1.0.70 `10-hooks.md`: Windows starts `.exe` paths; extensionless paths need Git Bash |
| Grok starts the hook directly (hook parent pid is the Grok process) | Documented only (a path command "starts it directly"); runtime check `UNVERIFIED` | no signed-in grok |

## Local targets

Install of the packaged layout and a no-Python hook run are recorded from workflow `probe-grok-acceptance.yml`
(run 41975051827 annotations) and `ci-artifacts.yml` (packaged-layout test, task 5.10).

| Step | macOS arm64 | macOS x64 | Linux x64 | Windows x64 |
|---|---|---|---|---|
| Install from packaged layout with real grok, hook copied | CI-real-grok-install | CI-real-grok-install | CI-real-grok-install (also local) | CI-real-grok-install |
| Installed/packaged hook runs with no Python on PATH, one event, stdin open, one line, empty stdout | CI-test (`tests/packaged.rs`) | CI-test | CI-test, local-test | CI-test |
| Logged `pid` equals the test process (parent semantics) | CI-test | CI-test | CI-test, local-test | CI-test |
| Session start, prompt, question, stop, end through a real Grok session | UNVERIFIED: grok not signed in | UNVERIFIED | UNVERIFIED | UNVERIFIED |
| Dots and explorer follow in Made from real Grok events | UNVERIFIED: no real session | UNVERIFIED | UNVERIFIED | UNVERIFIED |
| Logged `pid` equals the real Grok process (0.3) | UNVERIFIED | UNVERIFIED | UNVERIFIED | UNVERIFIED |

## Remote targets (one per supported target)

| Step | Linux x86_64 | Linux aarch64 | macOS x86_64 | macOS arm64 |
|---|---|---|---|---|
| Hook built per target under glibc 2.31 / macOS 12, checked, manifest-recorded, staged as the 10th artifact | CI-test (`ci-ssh-remote.yml`) | CI-test | CI-test | CI-test |
| Consented install, native hook ingested on a PATH without python3 (loopback, fixture grok) | CI-test (`tests/ssh/parity.py grok-parity`, Linux runner only) | UNVERIFIED: loopback job runs x86_64 only | UNVERIFIED: no macOS loopback server | UNVERIFIED |
| Real remote grok session producing hook lines | UNVERIFIED: grok not signed in | UNVERIFIED | UNVERIFIED | UNVERIFIED |

The Linux loopback job is regression evidence for the protocol and harness surface only, not a substitute for the
approved client/server matrix or native GUI evidence.
