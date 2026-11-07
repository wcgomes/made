# Design: agent-integration-modules

Non-normative. Guarantees live in `spec.md`. Line references are to `0b57210` (branch `agent-integration-modules`, identical to `worktree-claude-code-integration` at that commit), paths relative to `crates/`. Code below is an illustrative sketch; final names and signatures are settled in `tasks.md` and during implementation, as long as the spec holds.

## Approach

Today the integration set exists six times: `CompatibleTui` (`made-platform/src/lib.rs:2513-2546`), `AgentIntegration` plus `V1_SCOPE` (`made-app/src/integration_status.rs:873-925`), `TerminalTarget` (`made-app/src/tui.rs:33-50`), local `RootSource` (`made-app/src/session_follow.rs:74-105`), remote `RootSource` (`made-app/src/remote_workspace.rs`) and `LaunchTarget` (`made-remote/src/protocol.rs`). Around them sit exhaustive matches, per-integration boolean fields and hard-coded precedence chains in `workbench.rs`, `app.rs`, `ui.rs`, `diagnostics.rs` and `session_follow.rs`. The chosen structure keeps everything compile-time and replaces the local representations with one list:

```rust
// crates/made-app/src/integrations/mod.rs
pub type IntegrationRef = &'static dyn Integration;
pub type IntegrationList = &'static [IntegrationRef];

/// The product's integrations, in catalog (menu) order. The only place the
/// product names them. Precedence is the declared rank, not this order.
pub const PRODUCT: IntegrationList = &[&opencode::OPENCODE, &grok::GROK, &claude_code::CLAUDE_CODE];
```

Alternatives considered:

| Option | Extension cost | Verifiable by a test-only fake | Risk | Verdict |
|---|---|---|---|---|
| Dynamic plugins (dylib, WASM, scripting) or manifest/config-driven integrations | low at run time | yes | ABI, loading, versioning, signing and security surface; user non-goal | rejected |
| Closed enum with `match` (today) or `enum_dispatch` | new variant plus every match | no: a test cannot add a variant | low per change, high cumulative | rejected |
| Data-only descriptor table, behavior stays in core matches | low for data, full for behavior | partly | follow policy and launch preparation stay as core matches | rejected |
| Compile-time type list (tuple of integrations, generic `Workbench<L>`) | one type in a tuple | yes | generics leak through `Workbench`, `AppModel`, UI; long compile errors | rejected |
| Static slice of trait objects plus const descriptors | one module plus one list line | yes, via an injected list | one virtual call per seam per frame at three entries; downcast confined to modules | chosen |

The trait-object list is the smallest structure that meets AIA-04: a test crate can implement the trait and pass its own list to the same constructors the product uses. Being a `const` `&'static` slice of unit-struct values, it is immutable, needs no registry or initialization order, and can be used in other constants such as `V1_SCOPE` (see Identity and descriptor).

## Architectural boundaries

Module layout in `made-app` (final names in `tasks.md`):

```
crates/made-app/src/integrations/
├── mod.rs              # Integration trait, Descriptor, Mark, Rank, EnvPolicy, FollowKind,
│                       # FollowPolicy, DiagnosticsDecl, IntegrationRef/List, PRODUCT list,
│                       # Slots<T>, list validation (unique keys and ranks)
├── shared/
│   └── process_ancestry.rs   # process_belongs_to_with and friends (from grok_follow.rs)
├── opencode/
│   ├── mod.rs          # OPENCODE descriptor + impl Integration
│   ├── launch.rs       # composed-config env contribution (bridge start stays in made-bridge)
│   ├── tab.rs          # per-tab state: bridge handle, config guard, user-config capture
│   └── diagnostics.rs  # primary status, opencode-missing memory, version support
├── grok/
│   ├── mod.rs          # GROK descriptor + impl Integration (args ["dashboard"])
│   ├── follow.rs       # from grok_follow.rs (inventory, hook log, ownership)
│   ├── plugin.rs       # from grok_plugin.rs (install worker)
│   ├── tab.rs          # per-tab state: GrokLiveSession, hook offsets, activities
│   └── diagnostics.rs  # GrokDiagnosticsPhase/Hold/RunningHold, row (from app.rs, diagnostics.rs)
└── claude_code/
    ├── mod.rs          # CLAUDE_CODE descriptor + impl Integration
    ├── follow.rs       # from claude_follow.rs (spool, ClaudeTab, ownership, liveness)
    ├── launch.rs       # plugin-list composition and spool env (from tui.rs and made-platform)
    ├── tab.rs          # ClaudeRuntime (from workbench.rs)
    └── diagnostics.rs  # CLAUDE_GRACE, phase, inputs, rows (from integration_status.rs)
```

Assets stay in `crates/made-app/assets/` and are referenced by `include_bytes!` from the owning module. Channel code stays in its crate: `made-bridge` (OpenCode), `made-terminal` OSC 7501 scanning (`emulation.rs`, `PROTOCOL_APP`), `plugins/*`, the hook crates. Rule of thumb: the core may name a type from `integrations/mod.rs` and nothing from an integration's own folder; an integration folder may name `integrations/mod.rs`, `integrations/shared/` and core services, never another integration's folder.

### Identity and descriptor

```rust
pub struct Descriptor {
    pub key: &'static str,            // unique internal key: "opencode", "grok", "claude-code"
    pub program: &'static str,        // PATH executable: "opencode", "grok", "claude"
    pub label: &'static str,          // "OpenCode", "Grok Build", "Claude Code"
    pub menu_item_id: &'static str,   // new_tab_menu_item_id action: "opencode", "grok", "claude"
    pub scope_id: &'static str,       // "opencode-v2", "grok-build", "claude-code"
    pub scope_name: &'static str,     // "OpenCode v2", "Grok Build", "Claude Code"
    pub root_source_id: Option<&'static str>, // "opencode", "grok", "claude-code"
    pub mark: Mark,                   // Vendored("opencode.svg") | Image { uri, bytes }
    pub rank: Rank,                   // 100 OpenCode, 200 Claude Code, 300 Grok Build
}
```

There is no transitional `CompatibleTui` type: the identity group migrates every caller to `IntegrationRef` in one atomic group, with variants replaced by the upper-case constants `OPENCODE`, `GROK` and `CLAUDE_CODE` and `CompatibleTui::ALL` replaced by `PRODUCT`, which keeps the same catalog order. This avoids an associated-const shim, whose variant-style names would need a lint suppression, which this change does not use anywhere. Transitional code is limited to public accessors and `pub use` re-exports between `// BEGIN transitional-shim (AIA-11)` and `// END transitional-shim` markers, which trigger no lint and are exempt from neutrality scans until step 3 removes them.

Several identifiers exist today with different values for the same integration (menu `claude`, root source and scope `claude-code`); keeping each as its own field is what keeps every literal stable. `IntegrationRef` implements `PartialEq`, `Eq` and `Hash` through `descriptor().key`, so `TerminalTarget::Tui(IntegrationRef)` stays `Copy + Eq`. `CompatibleTui` derives `Ord` by discriminant; any ordering use moves to list position. `V1_SCOPE` stays a `pub const ScopeBoundary`, so the existing `const { assert!(...) }` uses of `chat_ui`, `worktree_management` and `separate_opencode_server` (`tests/scope_boundary.rs:49, 70, 142`, `tests/degraded.rs:511-513`) keep compiling unchanged; its `agent_integrations` field changes type from `&'static [AgentIntegration]` to `IntegrationList` and is defined as `agent_integrations: PRODUCT`, which is the derivation. Readers of `id()`/`name()` (`tests/degraded.rs:503-510`, `tests/scope_boundary.rs:29-44`) adapt to `descriptor().scope_id`/`scope_name`, and the index asserts compare descriptor keys against `OPENCODE`, `GROK`, `CLAUDE_CODE`.

`made-platform` stops naming integrations: `detect_compatible_tuis_in_path` becomes a lookup over program names supplied by the caller (`made-app` passes the list's `program` fields and maps hits back to `IntegrationRef`); `AppPaths::grok_plugin_dir` and `claude_plugin_dir` (and the matching `PathsInputs` fields) become one accessor for an app-owned plugin package by directory name under the install root (`plugins/grok-made`, `plugins/claude-code-made`); `CLAUDE_CODE_PLUGIN_DIRS_ENV`, `claude_plugin_dirs_separator`, `compose_claude_plugin_dirs` and the safety check in `shipped_claude_plugin_dir` move to `integrations/claude_code/launch.rs` (pure code over `PlatformFamily`). Kept in `made-platform` as OpenCode channel code: `plugin_dir` with its development fallback and `PluginDirSource`, `opencode_config_path`, the OpenCode version probe in `detach.rs`. `is_sensitive_env_key` stays central (decision in proposal).

### The interface

```rust
pub trait Integration: Sync + 'static {
    fn descriptor(&self) -> &'static Descriptor;

    // Launch
    fn launch_args(&self) -> &'static [&'static str] { &[] }       // Grok: ["dashboard"]
    fn env_policy(&self) -> EnvPolicy { EnvPolicy::NONE }          // { shell: bool, own_direct: bool, strip: &[&str] }

    // App-wide state and detection
    fn new_app_state(&self, init: &AppInit) -> Option<Box<dyn AppState>> { None }

    // Per-tab state (local tabs at creation; remote tabs where declared)
    fn new_tab_state(&self, target: TerminalTarget, init: &TabInit) -> Option<Box<dyn TabState>> { None }
    fn polls_remote_tabs(&self) -> bool { false }                  // Claude Code: true (OSC 7501 status)

    // Follow and presentation
    fn follow(&self) -> FollowKind { FollowKind::None }            // SessionChannel | Directory(FollowPolicy) | None

    // Diagnostics
    fn diagnostics(&self) -> DiagnosticsDecl { DiagnosticsDecl::NONE } // Primary | Row { report_keys, initial } ; smoke marker
}

pub trait AppState: Any {
    fn on_detection(&mut self, detected: bool, ctx: &mut AppCtx);    // Grok: spawn plugin ensure; OpenCode: remember missing
    fn poll(&mut self, ctx: &mut AppCtx) {}                          // Grok: plugin-ensure receiver
    fn row(&self, ws: &WorkspaceView) -> Option<RowReport> { None }  // pill + sentence
    fn smoke_marker(&self) -> Option<(&'static str, String)> { None }
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

pub trait TabState: Any {
    fn prepare_launch(&mut self, target: TerminalTarget, ctx: &mut LaunchCtx) -> EnvContribution; // set + strip, log lines
    fn on_spawned(&mut self, pid: Option<u32>) {}                     // Claude: install ClaudeTab with pid
    fn poll(&mut self, now: Instant, io: &mut TabIo) -> TabReport;   // directory report, presence
    fn on_follow_outcome(&mut self, outcome: &SessionFollowOutcome) {} // Claude: applied/rejected memory
    fn presentation(&self, target: TerminalTarget) -> Option<Presentation>; // brands?, sessions/dots, tooltip title
    fn teardown(&mut self) {}                                         // today: self.claude = None
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
```

Defaults let an integration that needs no follow, no state or no row stay small. OpenCode's bridge events are drained through its `TabState::poll`, but at the session-channel position of the tick (see Flow) so the order of side effects does not change.

### Runtime state

Adapters own their state from step 1: every integration, including the ones whose channel code is relocated later, is a module whose structs hold its current per-tab and app-wide state, moved wholesale (logic unchanged) out of `Workbench` and `AppModel`. Steps 2 and 3 then only relocate the remaining channel-logic files of Grok and OpenCode into their folders and remove shims; no step keeps integration state in the core.

`Slots<T>` is a `Vec<Option<T>>` parallel to the list the owner was built with. `AppModel` holds `Slots<Box<dyn AppState>>` and each `Workbench` holds `Slots<Box<dyn TabState>>`; the list itself is stored next to them (the product list, or the injected one). An integration module reaches its own concrete state with a typed helper on its own descriptor, for example `claude_code::tab(&workbench) -> Option<&ClaudeTab>`, which finds its slot by key and downcasts; a failed downcast is a programming error caught by tests. This replaces today's `claude`, `claude_plugin`, `grok_*` and OpenCode bridge fields in `workbench.rs:589-623, 873-898` and the `grok_phase`, `grok_plugin_receiver`, `claude_phase` and OpenCode-missing fields in `app.rs:460-497, 570-624`.

Polls return a `TabReport` (directory report, presence change) instead of calling the follow coordinator, because today `poll_claude` must drop its runtime borrow before `follow_claude_path` (`workbench.rs:4466-4481`). The core applies the report to the coordinator and hands the outcome back through `on_follow_outcome`, which keeps Claude Code's applied/rejected-directory memory and its retry-at-liveness rule (`workbench.rs:4467-4480`) inside the Claude Code module.

Per-tab context the core passes to a poll (`TabIo`) exposes only what integrations use today: terminal pid, `is_running`, `take_protocol_events`, `take_cwd_report`, the process-parent map on demand, the diagnostics log and app paths. This keeps the dependency direction inward: integration modules depend on core services, the core depends only on `integrations/mod.rs`.

### Public test surface (settled in step 3)

The external test crate reaches an integration through its module only; the old core paths (`made_app::claude_follow`, `made_app::tui` launch items, `made_app::integration_status` Claude Code and OpenCode items, `made_app::app::Grok*Hold`, `Workbench::claude_tab`, `Workbench::note_opencode_missing`, `AppModel::hold_*grok*`, `IntegrationState::note_opencode_*`, `workbench::MAX_USER_CONFIG_BYTES`) are gone. `integrations::{claude_code, grok, opencode}` are `pub mod`; everything else in them stays `pub(crate)`.

| Module | Public items used by the tests |
|---|---|
| `claude_code::follow` | the channel code that was `claude_follow` (`parse_record`, `ClaudeTab`, `ClaudeActivity`, `SpoolReader`, `SpoolInstance`, `TabSpool`, `shared_instance`, limits), unchanged |
| `claude_code::launch` | `claude_launch_env` (free function over the target), `ClaudeLaunchEnv`, `CLAUDE_SPOOL_ENV`, `CLAUDE_CODE_PLUGIN_DIRS_ENV`, `claude_plugin_dirs_separator`, `compose_claude_plugin_dirs`, `shipped_claude_plugin_dir` |
| `claude_code::diagnostics` | `claude_diagnostics_row`, `remote_claude_diagnostics_row`, `ClaudeDiagnosticsInputs`, `ClaudeDiagnosticsPhase`, `CLAUDE_GRACE` |
| `claude_code` | `#[doc(hidden)] tab(&Workbench) -> Option<&ClaudeTab>` |
| `grok` | `GrokDiagnosticsHold`, `GrokRunningHold`, `#[doc(hidden)] hold_diagnostics(&mut AppModel, ..)`, `#[doc(hidden)] hold_workspace_running(&mut AppModel, ..)` |
| `opencode` | `MAX_USER_CONFIG_BYTES`; `opencode::diagnostics::{is_supported_version, note_missing, note_version, note_exited, SUPPORTED_OPENCODE_MAJOR}` over `&mut IntegrationState` |

The core learned only neutral hooks, so it names no integration module (the import scan covers all three): `TabState::{start, shutdown, after_shutdown, bridge, token}` (the bridge start, the user-configuration capture and its verification belong to the session-channel owner's tab state), `AppState::{holds_detection, on_workspace_closed, take_pending_notice, notify_workspace}` with `WorkspaceNotices` (one announcement mechanism for the Grok "hooks unavailable" line and the OpenCode "missing" status), `Integration::{configures_launch, supports_peer_version}`, and the list helpers `session_channel_state`, `session_bridge`, `session_token` and `session_peer_supported`, all bound to the entry whose `follow()` is `SessionChannel`. `session_follow.rs` already found that entry through `FollowKind` and names no integration.

## Flow

Tick order is preserved exactly (`workbench.rs:3923-4060`):

```
tick(local tab):
  drain session channel            # OpenCode bridge events, as drain_bridge_events today (~4020)
  poll_shell_root_revalidation     # core, unchanged position
  for entry in list where follow != SessionChannel:   # list order = Grok, then Claude Code
      report = slot.poll(now, io)  # Grok: hooks then directory; Claude: terminal reports, spool, liveness
      apply report to coordinator; slot.on_follow_outcome(outcome)
  poll_worktree, poll_git_status   # core, unchanged
tick_remote(remote tab):
  ... unchanged remote polls ...
  for entry in list where polls_remote_tabs: lazily create tab state, poll   # Claude Code only today
```

Today Grok polls before Claude Code (`poll_grok_hooks`, `poll_grok_directory`, `poll_claude` at `workbench.rs:4043-4045`), which is list order, so iterating the list keeps it. The `contributions` line below is the order in which they are gathered, not the order in which they apply. Launch (`workbench.rs:1164-1286`):

```
start_terminal(target):
  remote target -> return (unchanged)
  options = target.pty_options(cwd, PATH)            # program from descriptor, args from launch_args
  contributions = for entry in list (gathering order): covered = entry.env_policy().covers(entry, target)
                    c = slot.prepare_launch(target, covered, ctx)           # Claude: spool + compose; logs as today
                    c = c if covered else policy.strip only                  # the core applies the declared strip
  options = apply core strips (SHELLOPTS, BASH_ENV, ENV), then contributions from the last list entry to the first (earliest entry wins)
  spawn; on success: for entry in list: slot.on_spawned(pid)
  on failure: retry in fallback dir with the same contributions; on_spawned on success
```

OpenCode's composed configuration is produced when the tab starts its bridge, before `start_terminal` (`workbench.rs:~1030-1161`); its `prepare_launch` only returns the stored content for targets its policy covers, so the bridge start and its log lines keep their position. Contributions apply from the last list entry to the first, so the earliest entry wins on a shared variable (pinned by `contributions_apply_last_entry_first_so_the_earliest_entry_wins`); the integrations' variables are distinct, so the resulting environment is identical, and the only order-sensitive output, the log lines, stays where `prepare_launch` and spawn emit them today (the launch-environment and log-order characterization tests pin this).

## Significant decisions

### Precedence

Ranks are small integers with gaps (OpenCode 100, Claude Code 200, Grok Build 300; lower wins) so an integration can be inserted between two others without renumbering. `effective_root_source` becomes: the session-channel owner if it holds (active session or effective view), else the lowest-ranked directory-follow holder, else `Shell`. `release_to_fallback` (`session_follow.rs:631-678`) walks directory holders by rank and takes the first usable one, which is today's Claude-then-Grok order. `follow_shell_cwd` (`session_follow.rs:536`) checks "any directory holder" instead of naming two. `RootSource` becomes `Shell | Agent(IntegrationRef)` and `as_str` returns `root_source_id`, so diagnostics keep `shell`, `opencode`, `claude-code`, `grok`. `is_agent_owned` (`session_follow.rs:373-375`, used at `workbench.rs:4010, 4128, 4524`) becomes "the session channel owns the root", its exact meaning today. List validation (a test over the product list and every test list, plus a `debug_assert` at construction) rejects duplicate keys and ranks and requires the session-channel owner to have the lowest rank.

Local tab presentation (`ui.rs:974-1004, 1777-1878`) iterates integrations by rank and takes the first whose `presentation` says it brands the tab: OpenCode active (connected bridge, or its direct tab running), then Claude Code present, then Grok open, else folder. The remote exception, where a remote tab that presents Grok Build under SSH-10 is not branded by Claude Code (`workspace_claude_branded`, `ui.rs:1000-1004`), is remote presentation and stays an explicit rule on the remote path until the remote change harmonizes it.

### Follow policy

```rust
pub enum FollowKind { None, SessionChannel, Directory(FollowPolicy) }
pub struct FollowPolicy {
    pub on_unusable: OnUnusable,            // Reject { detail } | Ignore
    pub under_session_channel: WhileOutranked, // Retain | Ignore
    pub under_higher_directory: WhileOutranked, // Retain | Ignore
}
// Claude Code: { Reject{"Claude Code directory is not usable"}, Retain, Retain }
// Grok Build:  { Ignore, Ignore, Retain }
```

Branch table that the generic `follow_directory(entry, report)` must reproduce (from `session_follow.rs:551-620`; "holds" means the integration's retained directory is set):

| State | Report | Claude Code today | Grok Build today |
|---|---|---|---|
| session channel holds | usable dir | retain dir; `ActiveSession`, no switch | ignore entirely; `ActiveSession`, no switch |
| session channel holds | unusable dir | `RejectedSnapshot`, dir unchanged | ignore; `ActiveSession` |
| session channel holds | release, had dir | clear dir; `ActiveSession` | ignore (dir kept); `ActiveSession` |
| session channel holds | release, no dir | `NoActiveSession` (checked before the channel) | `ActiveSession` |
| higher directory holder holds | usable dir | n/a (nothing outranks Claude Code but the channel) | retain dir; `NoActiveSession`, no switch |
| higher directory holder holds | unusable dir | n/a | ignore; `NoActiveSession` |
| higher directory holder holds | release | n/a | clear dir; `NoActiveSession` |
| no higher-ranked holder | usable dir | set dir; switch if root differs; `NoActiveSession` | same |
| no higher-ranked holder | unusable dir | `RejectedSnapshot`, dir unchanged | `NoActiveSession`, dir unchanged |
| owns | release, had dir | clear; `release_to_fallback` | clear; `release_to_fallback` |
| nothing held | release | `NoActiveSession`, no switch | `NoActiveSession`, no switch |

The order of checks differs (Claude Code tests "had a directory" before "channel holds"; Grok tests the channel first). The generic function encodes the order per policy rather than unifying it; the characterization test enumerates every row with generation and retained-directory assertions and is written against today's named functions first. If encoding a row as policy turns out to need a flag that only one integration uses, that flag is still declared policy, never a branch on the integration's identity.

### Launch

`EnvPolicy { shell, own_direct, strip }` states where an integration's variables go. Declared values: OpenCode `{ shell: true, own_direct: true, strip: [] }` (composed configuration via `CONFIG_CONTENT_ENV`); Claude Code `{ shell: true, own_direct: true, strip: ["MADE_CLAUDE_SPOOL"] }` with the spool also stripped on its own targets when no spool is bound (`with_claude_launch_env`, `workbench.rs:5833-5849`); Grok `NONE` plus `launch_args = ["dashboard"]`. `TerminalTarget::loads_claude_plugin` (`tui.rs:84-86`) and the composition gate in `start_terminal` (`workbench.rs:1168-1181`) become one generic predicate, "target is the shell or this entry's own direct target, and the policy covers it". Remote targets return before composition as today.

### Diagnostics

`DiagnosticsDecl` is `Primary` for OpenCode (it owns `status`, `reason code`, `reason`) or `Row { status_key, sentence_key, initial }` for a secondary row: Grok `{"grok status", "grok", NOT_ON_PATH}`, Claude Code `{"claude status", "claude", Inactive + "Claude Code is not on PATH."}` (`diagnostics.rs:634-637, 1184-1187`). The snapshot's `grok_pill`, `grok_sentence`, `claude_pill`, `claude_sentence` become a list-ordered `rows` collection; `to_text`, the About card (`ui.rs:3360-3366, 3580-3592`), the repaint hash and `push_grok_diagnostics` (`app.rs:1090`, renamed) iterate it. Remote setters (`set_remote_grok`, the remote Claude Code row) address a row by key. Smoke markers (`app.rs:2847`) iterate the list: OpenCode returns `opencode_plugin`, Grok returns `grok_status`, Claude Code returns none, so the line is unchanged. `GrokDiagnosticsPill` becomes one neutral pill type with the same labels; the `ClaudeDiagnosticsPill` alias goes.

### Shared hook library

New workspace crate `crates/made-hook-common` (library only, `publish = false`, dependencies `serde_json` and the same `windows-sys` features under `cfg(windows)`; no `libc`, which stays in `made-claude-hook` for its Unix pid chain). The only `windows-sys` use in either hook today is the Toolhelp parent lookup (`made-grok-hook/src/main.rs:88-92`, `made-claude-hook/src/main.rs:256-260`); once it moves, each hook manifest drops its direct `windows-sys` dependency unless another use remains, and the dependency comes transitively through `made-hook-common`. Contents: `Chunk`, `READ_DEADLINE`, `POLL_SLICE`, `spawn_stdin_reader`, `format_timestamp`, `json_string`, `field_text`, `text_of`, a `run_silently(fn)` panic-silent main wrapper, and the Windows Toolhelp `parent_of`. `read_event_with` diverges (Claude Code adds `MAX_INPUT` 8 MiB and a `}` completion gate, `made-claude-hook/src/lib.rs:18, 53`); it moves only as `read_event_with(read_chunk, deadline, limits)` with Grok passing "no cap, no gate" if the existing tests of both hooks and the golden corpus pass unchanged, otherwise each hook keeps its copy. Each hook crate re-exports moved public items (`pub use made_hook_common::{Chunk, READ_DEADLINE, format_timestamp, ...}`) so `tests/` keep their paths; duplicated helpers in `tests/common/mod.rs` move to a dev-only shared helper only if both copies are identical.

Unchanged: binary names `made-grok-status` and `made-claude-status`, `autobins = false`, status mapping, record format, output target (Grok JSONL log, Claude Code spool), title sanitization, pid chain, limits, staging scripts, packaging and companion builds. Companion builds compile the hook inside `rust:1-bullseye` with the workspace mounted, so a path dependency inside the workspace builds there; the glibc 2.31 and macOS 12 checks apply to the same binaries. `cargo tree -p made-grok-hook -e normal --prefix none` and the same for `made-claude-hook`, per target family, with the `made-hook-common` node filtered out, must list the same external crates before and after.

## Seam inventory

Survey item numbers refer to the seam survey made for this proposal (job scratch file `integration-seams-survey.md`, not stored in the repository). "List" means the core iterates the list; "decl" means a descriptor field or trait method; "module" means it moves into the integration's folder; "stays" means out of scope or channel code.

| # | Seam today | After |
|---|---|---|
| 1-2 | `CompatibleTui`, `ALL`, `program`, `label`; detection over `ALL` | decl (descriptor) + list; platform detection over supplied program names |
| 3-5 | `AppPaths` per-integration plugin dirs; `shipped_claude_plugin_dir`; `CLAUDE_CODE_PLUGIN_DIRS` composition | generic plugin-package accessor; Claude Code parts to `claude_code/launch.rs` |
| 6 | `is_sensitive_env_key` OpenCode literals | stays (central redaction list) |
| 7 | `opencode_config_path` | stays (OpenCode channel code in platform) |
| 8-10 | `pty_options` Grok args; `loads_claude_plugin`; `claude_launch_env`, `CLAUDE_SPOOL_ENV` | decl (`launch_args`, `env_policy`) + `claude_code/launch.rs` |
| 11 | remote `TerminalTarget` arms | stays (remote) |
| 12-16 | `RootSource`, precedence, `grok_root`/`claude_root`, follow functions, `release_to_fallback`, `is_agent_owned`, `as_str` | ranked directory holders + declared `FollowPolicy`; `RootSource::Agent` |
| 18-27 | `ClaudeRuntime`, `GrokLiveSession`, `grok_*`/`claude*` fields, start-path Claude Code calls, tick polls by name, follow paths, teardown | `Slots<TabState>`; `prepare_launch`/`on_spawned`/`poll`/`on_follow_outcome`/`teardown`; modules |
| 28 | per-integration diagnostics rows into the snapshot | list of declared rows |
| 29-32 | remote flags, probe, `arm_remote_client` | stays (remote) |
| 33, 42 | `compatible_tuis` options, `begin_tui_workspace` | already generic; take `IntegrationRef` |
| 34-36 | Grok and Claude Code phases, `initial_*_phase`, AppModel fields | `Slots<AppState>`; modules |
| 37-41 | diagnostics push on add-workspace; detection if-chain; `remember_opencode_missing`; Grok plugin ensure | list + `AppState::on_detection`/`poll`; modules |
| 43 | smoke markers | list + `smoke_marker` |
| 44, 50, 54 | `begin_remote_*`, remote `+` items, server conditions and consent card | stays (remote) |
| 45-49, 51 | branding predicates, marks, `WorkspaceTabInfo` flags, branding chain, `+` menu icon match, tab paint chains | rank-ordered `presentation` + descriptor mark; `WorkspaceTabInfo` carries the branding entry and its presentation |
| 52-53 | About rows, `grok_status_pill_ui` | list of rows; neutral name |
| 55 | snapshot fields and report lines | list of rows with declared keys |
| 56 | `AgentIntegration`, `V1_SCOPE` | derived from the list (`scope_id`, `scope_name`) |
| 57-58 | Claude Code diagnostics in `integration_status.rs`; OpenCode version support | modules |
| 59 | OSC 7501 `PROTOCOL_APP` | stays (channel code) |
| 60 | `STAGE_FILES` | stays (remote; see proposal) |

Core seams that a new local integration must not need to edit (AIA-04): `integrations/mod.rs` (beyond the one list line), `tui.rs`, `session_follow.rs`, `workbench.rs`, `app.rs`, `ui.rs`, `diagnostics.rs`, `integration_status.rs`, `made-platform` detection and app paths.

## Migration and rollback

Each step is one PR, merged only with every CI workflow green, and each is a pure code move with no persisted data or format change, so rollback is `git revert` of that PR. Old public paths keep `pub use` shims while tests still import them.

1. **Interface, all integrations as state-owning modules, and Claude Code channel code.** Phase 0 characterization tests come first, each driving behavior through a delimited test-only adapter block so later API changes touch only that block. Then step 1a, in commit groups that are each compile-green and test-green at their end: (G1) identity: `integrations/mod.rs` with the trait, descriptor, `Slots`, validation and `PRODUCT`, `IntegrationRef` replacing `CompatibleTui` and `AgentIntegration`, generic platform detection and plugin-package accessor, `V1_SCOPE` over the list, menu from descriptors; (G2) follow: ranked directory holders and declared policies in `session_follow.rs`; (G3a) an additive state skeleton that is green by itself: the state traits and `Slots` fields in `Workbench` and `AppModel`, with the generic launch, tick, remote-tick, detection and teardown loops at today's positions, all slots still empty; then (G3b, G3c, G3d) one small group per integration in the order Claude Code, Grok, OpenCode, each starting with an inventory of that integration's fields, methods and their reader files, moving its fields into its module state, and adding transitional accessors for every reader file found, so the core loops serve moved entries while not-yet-moved ones keep their explicit code until their group lands; (G4) presentation and diagnostics: rank-ordered branding, neutral pill type (the `ClaudeDiagnosticsPill` alias goes), rows, About, report, smoke markers, and removal of the transitional accessors whose readers moved to list iteration (the ones tests use stay until step 3); (G5) hidden test-support injection points, the fake integration and list-variation tests. Then step 1b relocates Claude Code channel code into `integrations/claude_code/` (`claude_follow.rs`, `tui.rs` env, platform composition, diagnostics), and `process_belongs_to_with` moves to `integrations/shared/`.
2. **Grok Build channel code.** `grok_follow.rs`, `grok_plugin.rs` and the Grok phase and row code from `app.rs` and `diagnostics.rs` are relocated into `integrations/grok/`, next to the state and polls the Grok module has owned since 1a; shims cover old paths. The branch table and all characterization tests stay green.
3. **OpenCode channel code and shim removal.** OpenCode-specific pieces of `integration_status.rs`, the bridge-start wiring, user-config capture and composed-config code, the active predicate and icon move into `integrations/opencode/`; the session channel in `session_follow.rs` stays core with OpenCode as its one owner. Shims are removed and test imports updated in path-only commits.
4. **`made-hook-common`.** Golden corpus first (record both hooks' outputs on today's code with an injected clock and ancestry), then the crate, the moves and the re-exports, then the dependency-tree check and packaged-layout tests on all targets.

5. **Renames and documentation.** The remaining scoped renames (`push_grok_diagnostics`, `grok_status_pill_ui`, `opencode_sessions_summary`, all in `app.rs` and `ui.rs`) and the wiki updates, in one small PR after step 4.

Step 4 does not depend on steps 1 to 3 in code; it follows the requested order. Reverting step N while keeping steps before it works because steps 2 to 5 only relocate files of one integration (or the hooks, or names and docs) behind the interface and state ownership introduced in step 1; reverting step 1 requires reverting 2 and 3 first.

## Test strategy

- **Existing suites are the gate:** `made-app` (`ui_model.rs`, `workspace_tabs.rs`, `session_follow.rs`, `launch_workspace.rs`, `diagnostics.rs`, `claude_*.rs`, `grok_plugin_install.rs`, `ssh_servers.rs`, `remote_workspace.rs`, `scope_boundary.rs`), `made-platform` (`contract.rs`, `ssh.rs`), `made-remote`, both hook crates, smoke and packaging workflows. Only path, accessor and constructor adaptations and added scanned sources are allowed.
- **Characterization tests first** (step 1), as listed above, kept for the whole change and after it.
- **Fake integration:** in `crates/made-app/tests/fake_integration.rs` (pins, validation and neutrality scans live in `crates/made-app/tests/integration_modules.rs`), a `FakeAgent` (key `fake-agent`, program `fake-agent`, label `Fake Agent`, rank 250, a directory-follow policy that retains while outranked, an `EnvPolicy` for shell and own direct, a diagnostics row `fake status`/`fake`) in an injected list `[OPENCODE, GROK, CLAUDE_CODE, FAKE]`. A second variant with rank 150 sits between OpenCode and Claude Code. The fake exercises every seam AIA-03 lists: menu rows, detection reaction (`AppState::on_detection`), launch arguments and environment, app-wide and per-tab slots, polling and teardown, follow ranking, tab presentation (label, mark, dots, tooltip, accessible name), diagnostics rows, About rows, copied report and smoke marker `fake_status`. Injection points, all `#[doc(hidden)]` and documented as test support, never reachable from configuration: `AppModel::with_integrations(list)` (the app model passes its list to every `Workbench` it creates), `SessionFollowCoordinator::with_integrations(list)`, the menu model builder taking a list next to the detected entries, `DiagnosticsSnapshot` rows built from a list, and the UI helpers that build `WorkspaceTabInfo` and the About rows from a list. List-variation tests run each seam with `PRODUCT`, `PRODUCT` plus the fake, and `PRODUCT` without Claude Code, and assert the output contains exactly the entries of the list, so a seam that does not iterate the list fails a committed test. Its name avoids every forbidden surface in `scope_boundary.rs`, and the new identifiers of this change are checked for forbidden substrings (for example `acp`).
- **Set pins (AIA-09):** derived tests iterate `PRODUCT`; one explicit pin test asserts every descriptor field of the three integrations and their order and ranks verbatim.
- **Source scans:** `scope_boundary.rs` keeps `include_str!` for fixed core files and walks `CARGO_MANIFEST_DIR/src/integrations` recursively with `std::fs::read_dir`, asserting at least three integration modules were read. `claude_privacy.rs` (today `include_str!` of `claude_follow.rs` and the hook `lib.rs`/`main.rs`, lines 124-145) walks `src/integrations/claude_code/` in step 1 and adds `made-hook-common/src/lib.rs` in step 4, and fails if it reads no file, so it cannot pass vacuously after a move.
- **Neutrality scans:** a deny-list of legacy local-seam identifiers (for example `CompatibleTui::`, `claude_root`, `grok_root`, `ClaudeRuntime`, `GrokLiveSession`, `loads_claude_plugin`, `poll_claude`, `poll_grok_`) and a rule that core files import no `integrations::<name>::` path. Exempt: the whole files `ssh_servers.rs`, `remote_workspace.rs` and `remote_session.rs`, and regions between `// BEGIN remote-path (AIA-02)` and `// END remote-path` markers in `tui.rs`, `workbench.rs`, `ui.rs`, `app.rs` and `diagnostics.rs` around the remote code named in AIA-02.
- **Characterization adapters:** each phase 0 test keeps its expectations outside one block delimited by `// BEGIN characterization adapter` and `// END characterization adapter`. The check is per block: only lines between the markers may differ from the phase 0 commit, and the phase 0 lines outside them stay identical; in shared test files (`diagnostics.rs`, `workspace_tabs.rs`) edits outside the phase 0 block are checked separately as path-only.
- **Hooks:** existing tests per hook unchanged (`contract.rs` ports of `status_test.py`, `status.rs`, `spool.rs`, `packaged.rs`, `stage_script.rs`, `hooks_json.rs`), plus the golden corpus and the `cargo tree` comparison.
- **Platforms:** Linux, macOS (x64, arm64) and Windows CI build and test, ConPTY and app smoke, Wayland/X11 smoke and the SSH companion workflows on every step; `made-hook-common` Windows code is compiled by the Windows job. The pre-existing `detach.rs` Windows warnings are not touched; no new warning may appear.

## Risks

- **Hidden ordering:** log lines, tick order and detection side effects are order-sensitive; mitigated by the characterization tests and by the explicit session-channel tick position.
- **Follow regressions:** the divergent branches are subtle (check order differs); mitigated by the branch table written first and by keeping per-row policy rather than unifying.
- **Borrowing in `Workbench`:** trait-object polls cannot borrow the coordinator; mitigated by returning reports and applying outcomes back.
- **API churn in tests:** many tests use `CompatibleTui`, `claude_tab()`, snapshot fields, `V1_SCOPE.agent_integrations`; mitigated by shims until step 3, path-only adaptation and characterization adapter blocks.
- **Downcast misuse:** confined to each module's typed helper and covered by tests.
- **Stacked branch churn:** PR #6 is still open; rebasing this change on later PR #6 commits can conflict in the same files; mitigated by starting step 1 only after PR #6 merges.
- **`--locked` builds:** companion and CI builds run `cargo build/test -p made-grok-hook --locked` (`.github/scripts/build-linux-companion.sh:59-63`); step 4 must commit the `Cargo.lock` entry for `made-hook-common` in the same PR.
- **Overlap with `grok-native-hook`:** that change is implemented but not archived; step 4 edits `made-grok-hook` internals only and re-runs its tests and packaged-layout checks.
- **Remote paths reading integration state:** remote code that reads Claude Code or Grok state today (remote Claude Code runtime in `tick_remote`, remote Grok projection, remote branding exception) uses the owning module's typed helpers inside delimited remote paths, exempt from neutrality scans (AIA-02); remote logic itself is not changed. At `0b57210`, `ssh_servers.rs`, `remote_workspace.rs` and `remote_session.rs` import none of `grok_follow`, `grok_plugin` or `claude_follow`.
- **Atomic groups:** G1, G2, G3b, G3c, G3d and G4 of step 1a each change a type or field set used across several files and cannot compile until their last task; G3a, G5 and the other steps are green per task. `tasks.md` marks the atomic groups and gives each a compile-green and a test-green checkpoint; splitting state ownership per integration keeps each atomic group small.
