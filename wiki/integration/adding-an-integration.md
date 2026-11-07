# Adding an Integration

An integration is an agent terminal UI Made can launch directly, follow and diagnose. Each one is one module under [`crates/made-app/src/integrations/<name>/`](../../crates/made-app/src/integrations/mod.rs) that implements the `Integration` trait, plus one line in the static `PRODUCT` list. Everything is compile-time: nothing is loaded, discovered or configured at run time, and this is not a plugin API. Rationale and rejected alternatives are in [ADR 0009](../decisions/0009-static-integration-modules.md); the normative requirements are in the [`agent-integration-modules` spec](../../specs/changes/agent-integration-modules/spec.md).

## Module structure

| File | Holds |
|---|---|
| `mod.rs` | The `Descriptor` const, the unit struct implementing `Integration`, and the app-wide and per-tab state constructors. |
| `tab.rs` | The `TabState` implementation: start, launch preparation, polling, shutdown, diagnostics row, presentation. |
| `launch.rs` | Launch-environment composition (OpenCode, Claude Code). |
| `follow.rs` | Directory-follow memory and report handling (Claude Code, Grok). |
| `diagnostics.rs` | Row sentences and pills. |
| `plugin.rs` | Plugin staging and install (Grok). |

Files are added only when the integration has that concern. Code used by two or more integrations lives in [`integrations/shared`](../../crates/made-app/src/integrations/shared/mod.rs) (today the process-ancestry helper), never in another integration's module. A module never depends on another integration's module. Channel code (the bridge crate, hook crates, plugin packages, the OSC 7501 scanner in `made-terminal`) stays where it is and is used by core code only through the module.

## The interface

`Descriptor` is const data, one field per identifier because the same integration has several identifiers with different literals:

| Field | Meaning |
|---|---|
| `key` | Unique internal key; list equality is by key. |
| `program` | Executable name resolved through `PATH` by detection. |
| `label` | Product label for the new-tab menu and an open tab. |
| `menu_item_id` | Identifier of the new-tab menu item. |
| `scope_id`, `scope_name` | Identifier and name in the v1 scope declaration. |
| `root_source_id` | Identifier reported when the integration owns the explorer root, if any. |
| `mark` | `Mark::Vendored` icon file name or `Mark::Image` owned bytes. |
| `rank` | Precedence rank. |

`Integration` supplies behavior; every method but `descriptor` has a neutral default.

- `launch_args`: arguments of a direct launch.
- `follow`: `FollowKind::None`, `SessionChannel` or `Directory(FollowPolicy)`.
- `env_policy`: where launch variables go and which are stripped.
- `new_app_state` and `new_tab_state`: optional runtime state (see State slots).
- `polls_remote_tabs`, `diagnostics`, `configures_launch`, `supports_peer_version`.

## Precedence by rank

`Rank(u16)` is the declared precedence; a lower rank wins. Today OpenCode is 100, Claude Code 200 and Grok 300, leaving gaps so a new entry can sit between two others without renumbering. Rank is independent of list position: `PRODUCT` order is only the new-tab menu order. `validate` rejects duplicate keys, duplicate ranks and a session-channel owner that does not have the lowest rank.

## Follow policy

`Directory(FollowPolicy)` declares what differs between integrations instead of the core checking identity:

| Field | Values |
|---|---|
| `on_unusable` | `Reject(detail)` records a rejection and keeps the root; `Ignore` drops the report silently. |
| `under_session_channel` | `Retain` keeps the reported directory so it can take the root later; `Ignore` drops the report. |
| `under_higher_directory` | The same choice while a higher-ranked directory holder owns the root. |

A report is checked against the session channel first, then against higher-ranked directory holders. Claude Code rejects unusable directories and retains in both cases; Grok ignores unusable directories and is ignored under the session channel but retains under a higher directory holder. Exactly one entry may own the session channel (the bridge-backed OpenCode), so a new integration uses `Directory` or `None`.

## Environment policy

`EnvPolicy` has `shell` and `own_direct` flags and a `strip` list. The core asks the tab state for a contribution only on a covered target (the shell, or the integration's own direct target); on any other target it discards the answer and removes the `strip` variables. Remote targets never receive local variables. Contributions apply from the last list entry to the first, so when two integrations set one variable the earliest entry sets it last and wins; this order is part of the launch contract.

## State slots

`Slots<T>` holds optional values parallel to the list: slot `i` belongs to entry `i`. App-wide state (`AppState`) lives in the app model and handles detection results, per-frame polling, publication to workspaces and teardown. Per-tab state (`TabState`) lives in the tab and is `Send` because tabs are prepared on a worker thread. Only the owning module touches its concrete state; the core goes through the traits. A poll returns a `TabReport` (a directory report or a release) and the core hands it to the follow coordinator.

## Diagnostics

`diagnostics` returns `None`, `Primary` (the OpenCode-style primary status; at most one entry) or `Row(RowDecl)`. A row declares its copied-report keys (`status_key`, `sentence_key`) and its `initial` pill and sentence before detection; the tab state's `row` answers afterwards. Rows, About rows and copied-report lines follow list order.

## Add an integration

1. Create `integrations/<name>/mod.rs` with a `Descriptor` const and an implementation of `Integration`; add `tab.rs` and other files as needed.
2. Pick a unique key and a rank not used by another entry. Give the descriptor `Mark::Image` with the integration's own bytes: `Mark::Vendored` names an icon file in the explorer icon inventory, so a vendored mark also needs an edit to that inventory (a core file). The test that every `PRODUCT` mark resolves covers both kinds.
3. Add three lines to `integrations/mod.rs`: the `pub mod <name>;` declaration (with `#[doc(hidden)]`, like the others), the const `pub const <NAME>: IntegrationRef = &<name>::<Type>;`, and the `<NAME>` entry in `PRODUCT` at the menu position you want.
4. Declare follow kind and policy, env policy and diagnostics row as needed; ship the integration's own assets and channel code.
5. Run the gates in [Build and test](../operations/build-and-test.md). The pin test `integration_modules.rs` lists every descriptor field of `PRODUCT` and fails until you update it for the new entry.

## What stays outside the guarantee

Adding an integration needs no edit to another integration's module or the core, with these exceptions:

- Remote and SSH code (`ssh_servers.rs`, `remote_workspace.rs`, `remote_session.rs` and the delimited remote paths in core files) is excluded from the neutrality rule and may use an integration's typed helpers directly. A remote launch, consent or companion protocol change still needs edits there.
- The central redaction key list (`is_sensitive_env_key` in [`made-platform`](../../crates/made-platform/src/lib.rs)), only when the new integration's environment value carries a secret the generic patterns do not match.
- Packaging, CI scripts and the release layout.
- Spec contract changes such as the `V1 Scope Boundary` declaration.
- A second owner of the bridge session channel.

## How the extension property is proved

[`fake_integration.rs`](../../crates/made-app/tests/fake_integration.rs) defines `FakeAgent` only in test code, absent from `PRODUCT`, and plugs it in through an injected `IntegrationList` at two ranks (one above Claude Code). It appears in every seam without a production edit: menu rows, detection reaction, launch arguments and variables, app and tab state, polling and teardown, follow ranking, tab presentation, diagnostics and About rows, copied report lines and smoke markers. Companion tests run each seam over the product list, the list plus the fake and the list without Claude Code and assert that the output is exactly the list's entries, so a seam that stops iterating the list fails a committed test. Copy this file's `FakeAgent` as the template for a new integration's tests.

Related: [ADR 0009](../decisions/0009-static-integration-modules.md), [Architecture](../architecture.md), [Terminal-first runtime](../runtime/terminal-first.md#root-precedence).
