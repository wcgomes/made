# Tasks: ssh-grok-home-fallback

Readiness: `analyze` must pass before implementation; review runs `verify` against the persisted spec.

## 1. Detection on the server probe
- [x] 1.1 Add a failing test: grok only in `$HOME/.grok/bin`, absent from the non-interactive PATH, is reported by the probe and never spawned (path: crates/made-platform/tests/ssh.rs, `probe_finds_grok_in_user_install_dir_missing_from_non_interactive_path`)
- [x] 1.2 Extend the probe script with the install-location fallback for grok only, claude unchanged (path: crates/made-platform/src/ssh.rs)

## Checkpoint
- [x] verify Grok installed only in the user's install location (probe side)

## 2. Companion detection shared by every consumer
- [x] 2.1 Add failing tests for the fallback, executable check and install mode (path: crates/made-remote/tests/grok.rs)
- [x] 2.2 Add the PATH-then-install-locations grok detection (path: crates/made-remote/src/grok.rs)
- [x] 2.3 Use it for capabilities, launch and install mode (path: crates/made-remote/src/main.rs)
- [x] 2.4 Use it for the session capability report (path: crates/made-remote/src/supervisor.rs)

## Checkpoint
- [x] verify Grok installed only in the user's install location (companion side)
- [x] verify PATH wins and install locations are tried in order
- [x] verify Grok SSH button and direct dashboard launch and Remote Grok detection, direct launch and sessionless branding still hold (crates/made-app/tests/ui_model.rs `grok_ssh_menu_row_and_consent_card_follow_per_server_detection`, `remote_grok_direct_launch_brands_before_a_session_and_exits`, `remote_grok_tab_shows_grok_build_branding_and_dots`; PATH-based, all pass)

## 3. Close gaps found by verify
- [x] 3.1 Add a test that a PATH grok wins over an install location and that `.grok/bin`, `.local/bin`, `.cargo/bin` are tried in order (path: crates/made-remote/tests/grok.rs `grok_detection_prefers_path_then_tries_install_dirs_in_order`; probe script: crates/made-platform/tests/ssh.rs `probe_prefers_path_grok_then_tries_install_dirs_in_order`)
- [x] 3.2 Add a test that the supervisor capability report and the launch resolve the same binary as detection (capability report covered: crates/made-remote/tests/supervisor_grok_capabilities.rs; install mode covered: tests/grok.rs `install_mode_finds_grok_in_user_install_dir_outside_path`; the Grok launch itself has no existing fixture and is not covered)
- [ ] 3.3 Accepted gap, optional: a test that the `Grok SSH` launch starts the fallback binary with `dashboard` (path: crates/made-remote/src/main.rs; needs a staged --stdio Open handshake fixture that does not exist; the launch resolves its program through the same shared detection function as the covered consumers)
