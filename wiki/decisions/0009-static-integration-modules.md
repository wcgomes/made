# 0009: Static Integration Modules

**Status:** Accepted; implemented by the change [`agent-integration-modules`](../../specs/agent-integration-architecture.md), delivered in steps 0 to 5.

## Context

OpenCode, Grok Build and Claude Code were each woven through `made-app`: closed identity enums in `made-platform`, per-integration flags and fields in the workbench and app model, and `match` arms in the menu, launch, follow, tab, diagnostics and smoke code. Precedence and the differences between integrations (how an unusable directory is handled, what happens while outranked) were encoded as scattered branches. Adding a fourth agent meant editing the core in many places, and a test could not add an integration without production edits. The goal was a behavior-preserving reorganization, with the extension cost verifiable by a test.

## Decision

Each integration becomes one module of `made-app` behind one compile-time trait. See [Adding an integration](../integration/adding-an-integration.md).

- **Static trait-object list.** `PRODUCT` is a `&'static [&'static dyn Integration]` in catalog (menu) order and the only place the product names its integrations. The core takes a list of that type, injectable in tests, and iterates it at every seam.
- **Descriptors for data, trait methods for behavior.** Identifiers, labels, program and mark are a const `Descriptor`; launch arguments, env policy, state, follow, diagnostics and presentation are trait methods with neutral defaults.
- **Ranks.** Precedence is a declared `Rank` (OpenCode 100, Claude Code 200, Grok 300), independent of list order. A list with duplicate keys or ranks, or whose session-channel owner is not the lowest rank, is invalid.
- **Declared follow policy.** The differences between Claude Code and Grok (unusable directory, behavior under the session channel and under a higher directory holder) are `FollowPolicy` data, never identity checks. The pre-change behavior is pinned by characterization tests written before any code moved.
- **Declared env policy.** Where launch variables go and which are stripped is `EnvPolicy`. Contributions apply last entry first, so the earliest entry wins on a shared variable.
- **State in parallel slots.** Per-app and per-tab state is held in `Slots` parallel to the list and touched concretely only by the owning module.
- **Generic detection.** `made-platform` detects program names instead of knowing a closed set of integrations.
- **Shared hook library.** The two native hooks share identical code through `made-hook-common`; each keeps its binary, mapping and output.
- **Compile-time only, not a plugin system.** Every integration is built into the binary. Nothing is loaded, discovered or selected by configuration at run time, and no workspace crate depends on a dynamic-loading library.

## Rejected alternatives

- **Dynamic plugins (dylib, WASM, scripting) or manifest/config-driven integrations:** loading, versioning, signing and security surface for a need the product does not have.
- **Closed enum with `match` or `enum_dispatch`:** every addition edits the enum and its matches, and a test-only fake cannot add a variant, so the extension property could not be verified.
- **Data-only descriptor table with behavior in core matches:** follow policy and launch preparation would stay as core branches.
- **Compile-time type list with a generic `Workbench<L>`:** generics leak through the workbench, app model and UI for no benefit at three entries.

## Consequences

- Adding a local integration that uses directory follow or no follow needs a new module, one line in `PRODUCT` and its own assets and channel code, and no edit to another module or the core.
- Not covered by that guarantee (AIA-04 exceptions): remote and SSH code and the companion protocol, which stay deferred and may name integrations directly; the central redaction key list when a new value carries an unmatched secret; packaging, CI scripts and the release layout; spec contract changes such as `V1 Scope Boundary`; and a second owner of the bridge session channel.
- The session channel remains a single owner with the lowest rank. A second owner is a separate design.
- Source-scan tests cover moved code and the pin test trips on any list or identifier change, so adding an entry updates the pin deliberately.
- Rule 4 of the architecture gains `made-hook-common` beside the two hooks; the hooks' direct `windows-sys` dependency moved there.

## Evidence

The change migrated in independently mergeable steps: step 0 characterization baselines on the unchanged code; step 1 interface, list and every integration's state plus Claude Code's channel code; step 2 Grok Build's channel code; step 3 OpenCode's channel code and removal of transitional shims; step 4 the shared hook library; step 5 scoped renames, wiki and verify. Each step was green on the full gates and had a revert check on a scratch worktree (revert applied cleanly, tree equal to the previous step's tip, `made-app` tests passing) for steps 1 to 4. The fake integration in [`fake_integration.rs`](../../crates/made-app/tests/fake_integration.rs) proves the extension property at two ranks and across every seam.

See [`design.md`](../../specs/changes/archive/20261107-agent-integration-modules/design.md), [`proposal.md`](../../specs/changes/archive/20261107-agent-integration-modules/proposal.md) (evidence and CI run ids) and [Architecture](../architecture.md).
