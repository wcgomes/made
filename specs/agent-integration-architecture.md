# Agent Integration Architecture Specification

## Purpose

This specification owns the internal structure guarantees for Made's agent integrations (OpenCode, Grok Build, Claude Code): behavior preservation across the reorganization, the standard integration module structure, the single static integration list, extension without core edits, declared precedence, follow, launch and diagnostics policies, set-pinning tests, the shared native-hook library, and incremental delivery. "Core" means `made-app` and `made-platform` code outside the integration modules. "Integration module" means the one module of the standard structure that represents one integration. User-visible integration behavior remains owned by the OpenCode, Grok and Claude Code specifications.

## Requirements

### Requirement: Behavior preservation (AIA-01)
The reorganization MUST NOT change any behavior observable by users, by copied or logged output, by the smoke harness, by remote servers or by packaging, as defined by the live contract for OpenCode, Grok Build and Claude Code. This covers at least: the new-tab catalog and `+` menu (rows, order, labels, marks, Terminal row, folder-picker fallback, detection as a metadata-only PATH lookup); direct launch (absolute program, arguments, cwd, fallback-directory retry); the launch environment of every target, including which targets receive OpenCode's composed configuration and Made's Claude Code plugin entry and spool binding and which do not or have it removed; tab label, mark, dots, tooltip title and accessible name; explorer root follow and precedence (locally OpenCode > Claude Code > Grok > shell; remote root sources and remote presentation unchanged); diagnostics rows, pill labels, sentences and row order, About rows, the copied diagnostics report text including the `grok status`, `grok`, `claude status` and `claude` lines, root-source identifiers and diagnostics log messages with their order for the same sequence of events; smoke-layout markers; plugin install, staging and packaging layout; SSH probe, consent, staging, companion protocol and remote launch; and the output of both native hooks. Every existing automated test suite MUST pass after each delivery step with its expected values unchanged. An existing test MAY be adapted only to follow moved or renamed code (imports, accessors, constructors, `include_str!` paths), to add scanned files or sources so that a source scan keeps covering moved code, or as AIA-09 requires, and MUST NOT be deleted unless its assertions move unchanged to another test. A source-scan test MUST NOT pass because the code it scanned moved out of its scope.

#### Scenario: Existing suites stay the acceptance gate
- Given the workspace test suites and CI workflows at the commit before a delivery step
- When the step is applied
- Then every suite passes, no expected value in an existing test changed, and any test diff consists only of path, accessor or constructor adaptations, added scanned sources or AIA-09 derivations

#### Scenario: Copied report unchanged
- Given the same workspace state, detection results and integration states before and after the change
- When the user copies the diagnostics report
- Then the text is byte-identical, including the order and wording of the `grok status`, `grok`, `claude status` and `claude` lines

#### Scenario: Smoke markers unchanged
- Given the smoke harness run on a build before and after the change with the same packaged layout
- When the layout markers line is printed
- Then it keeps `opencode_plugin=` before `grok_status=` with the same values, and still has no Claude Code marker

#### Scenario: Remote behavior unchanged
- Given a saved SSH server with remote OpenCode, Grok and Claude Code available
- When the user opens remote tabs of each kind
- Then probe, consent, staging, launch, remote root sources, remote tab presentation (Grok Build under SSH-10 before Claude Code under SSH-11) and remote diagnostics behave exactly as before

#### Scenario: Source scans keep their coverage
- Given the privacy and scope source scans that read integration and hook sources
- When code moves into integration modules or the shared hook library
- Then each scan reads the moved files, including every file of the integration module directories and the shared hook library, and fails if it finds none

### Requirement: Standard integration structure (AIA-02)
Each integration MUST be represented by exactly one integration module in `made-app` that follows one standard structure and implements one common compile-time interface. Through that module the integration MUST declare its identity and display data (stable identifiers, labels, program name, menu mark), its precedence rank, its launch arguments and launch-environment policy, its reaction to detection, its app-wide and per-tab runtime state, its follow channel and follow policy, its tab presentation (when it brands a tab, its dots and tooltip title), and its diagnostics contribution. Each integration's runtime state MUST be read and changed only by its own module and by the core through the common interface. An integration module MUST NOT depend on another integration's module; code used by two or more integrations MUST live in a shared location outside every integration module. Channel code (the OpenCode bridge crate, the hook crates, the plugin packages, the OSC 7501 scanner in `made-terminal`, OpenCode-specific paths and version probe in `made-platform`) MAY stay where it is and MUST be used by local core code only through that integration's module. Remote and SSH code is excluded from this neutrality rule and MAY use an integration module's typed helpers directly: the files `ssh_servers.rs`, `remote_workspace.rs` and `remote_session.rs` in `made-app`, and the remote code paths inside core files (the remote `TerminalTarget` arms, the remote tick and remote tab state of the workbench, the remote `+` menu items, server conditions, Grok consent card and remote branding exception of the UI, the remote launch functions of the app model and the remote diagnostics setters), which MUST be delimited as remote paths so that neutrality checks can exempt exactly them.

#### Scenario: One module per integration
- Given the `made-app` sources after the change
- When a reviewer looks for the code that defines OpenCode, Grok Build or Claude Code behavior in the local app
- Then each integration's declarations and runtime state are found in its own module implementing the common interface, and the core outside the delimited remote paths contains no match, flag or field named after one integration for the seams this change covers

#### Scenario: No cross-integration dependency
- Given the Claude Code module, which today reuses a process-ancestry helper from the Grok code
- When its dependencies are inspected
- Then it uses the helper from a shared location and references no other integration module

#### Scenario: Compile-time only
- Given the common interface
- When Made is built and run
- Then every integration is compiled into the binary and nothing is loaded, discovered or configured at run time
- And no direct dependency of any workspace crate provides dynamic loading

### Requirement: Single integration list (AIA-03)
Exactly one static list in code MUST name the integrations of the product, in catalog order: OpenCode, Grok Build, Claude Code. Every core seam that needs the set of integrations (menu rows, detection, launch environment composition, app-wide and per-tab state, polling, follow ranking, tab presentation, diagnostics rows, About rows, copied report lines, smoke markers and the scope declaration) MUST iterate that list, or a list of the same type passed in by a test, instead of naming integrations. The constructors that accept an injected list MUST be reachable only from code (hidden test-support entry points), never from configuration, environment, files or user input. There MUST NOT be a runtime registry, discovery, dynamic loading, external manifest, or selection of integrations by configuration, environment variable or file. No direct dependency of any workspace crate provides dynamic loading; transitive platform or graphics dependencies (for example `libloading` through the graphics stack) are out of scope. The product build MUST contain exactly these three integrations, consistent with `V1 Scope Boundary`.

#### Scenario: List order is catalog order
- Given OpenCode, Grok and Claude Code are all on PATH
- When the user activates `+`
- Then the menu is Terminal, OpenCode, Grok Build, Claude Code, and the order comes from the list

#### Scenario: Nothing outside the build adds an integration
- Given a configuration file, environment variable or directory that names another agent program
- When Made starts
- Then the set of integrations offered, launched, followed and diagnosed is still OpenCode, Grok Build and Claude Code

#### Scenario: Product list is exact
- Given a release or development build
- When its integration list is inspected by test
- Then it contains exactly OpenCode, Grok Build and Claude Code in that order and no test-only integration

### Requirement: Extension without core edits (AIA-04)
Adding a local integration that uses directory follow (or no follow) MUST require only a new integration module of the standard structure, one entry in the static list, and that integration's own assets and channel code. It MUST NOT require editing another integration's module or the core. Outside this guarantee, and allowed to need edits: remote and SSH code and the companion protocol (deferred); the central secret-redaction key list, only when the new integration's environment value carries a secret its generic patterns do not match; packaging and CI scripts and the release layout; spec contract changes such as `V1 Scope Boundary`; and a second owner of the bridge session channel. This MUST be verified by a fake integration defined only in test code, absent from the product list, plugged in through the same interface and an injected list, which appears in every seam AIA-03 lists (menu rows, detection reaction, launch environment, app-wide and per-tab state, polling and teardown, follow ranking, tab presentation including label, mark, dots, tooltip and accessible name, diagnostics rows, About rows, copied report lines and smoke markers; the scope declaration is the product list by definition) without any production code edit, at more than one rank. For each seam, a test with an injected list that lacks an entry MUST show that entry absent from that seam's output, so a seam that does not iterate the list fails a committed test.

#### Scenario: Fake integration in the menu
- Given an injected list OpenCode, Grok Build, Claude Code, fake, and detection reporting all four
- When the menu model is built
- Then the rows are Terminal, OpenCode, Grok Build, Claude Code, then the fake with its declared label, id and mark, and without detection of the fake its row is omitted

#### Scenario: Fake integration detection reaction
- Given an injected list with the fake, whose app-wide state records the detection result it receives
- When PATH detection finishes with and without the fake program present
- Then the fake receives exactly one detection reaction with the matching result, in list order after the three product integrations, and their reactions are unchanged

#### Scenario: Fake integration launch environment
- Given the fake declares launch arguments and an environment variable for the shell target and its own direct target only
- When a shell tab, a fake tab, an OpenCode tab and a Grok tab are started
- Then the shell and fake tabs carry the variable, the fake tab runs the fake program with its declared arguments, the OpenCode and Grok tabs do not carry it, and every other variable of those tabs is as before

#### Scenario: Fake integration state, polling and teardown
- Given a workspace tab started with the fake in the injected list
- When the tab ticks and then closes
- Then the fake's per-tab state is created once for the targets it declares, is polled on each local tick after the product integrations, and is torn down once when the tab closes

#### Scenario: Fake integration follow and presentation at its rank
- Given the fake declares a rank between Claude Code and Grok Build and a policy that retains its directory while outranked, and reports a usable directory in a tab where Grok also reports one
- When OpenCode connects, then disconnects, then Claude Code becomes live, then exits
- Then the root and tab presentation follow OpenCode, then the fake, then Claude Code, then the fake again, and Grok never takes the root while the fake holds it; while the fake brands the tab, the label, mark, dots, tooltip title and accessible name are the fake's declared values

#### Scenario: Fake integration above Claude Code
- Given the fake declares a rank between OpenCode and Claude Code instead
- When OpenCode disconnects while the fake, Claude Code and Grok all hold directories
- Then the fake owns the root and the tab presentation, Claude Code and Grok take it in that order as holders release, and the three product integrations keep their relative order

#### Scenario: Fake integration diagnostics
- Given the fake declares a diagnostics row with report keys `fake status` and `fake`
- When diagnostics are shown and copied
- Then the fake row appears after the Claude Code row in the About card and its two report lines follow the `claude` line, while every other line is unchanged

#### Scenario: Fake integration smoke marker
- Given the fake declares the smoke marker `fake_status`
- When the smoke layout markers line is built
- Then `fake_status=` follows `grok_status=` before the companion fields, and the rest of the line is unchanged

#### Scenario: A list without an entry drops it from every seam
- Given an injected list that omits Claude Code
- When the menu, launch environment, follow, presentation, diagnostics rows, About rows, copied report and smoke markers are produced
- Then none of them contains Claude Code output, and the remaining integrations' output is unchanged

#### Scenario: Fake absent from the product
- Given the product integration list
- When it is inspected
- Then the fake is not in it and no production source refers to it

### Requirement: Declared precedence (AIA-05)
Each integration MUST declare a precedence rank, and ranks MUST be unique within a list. The core MUST derive local explorer-root precedence and local tab-presentation precedence from these ranks, never from catalog order or a hard-coded chain. The declared ranks MUST reproduce today's order: root follow OpenCode > Claude Code > Grok > shell, and local tab presentation OpenCode active > Claude Code present > Grok open > folder. When a holder releases, the root MUST return to the highest-ranked remaining holder with a usable directory, else the shell. Remote root sources and the remote presentation rule stay as they are. Root-source identifiers MUST stay `shell`, `opencode`, `claude-code` and `grok`.

#### Scenario: Precedence by rank
- Given a local tab where OpenCode, Claude Code and Grok each hold a usable directory
- When holders release one at a time in the orders OpenCode, Claude Code, Grok and OpenCode, Grok, Claude Code
- Then the effective root source is always the highest-ranked remaining holder, then `shell`, with the same root, generation and switch results as before the change

#### Scenario: Rank is independent of catalog order
- Given the catalog order OpenCode, Grok Build, Claude Code
- When Claude Code and Grok both hold a directory and OpenCode holds none
- Then Claude Code owns the root and the tab presentation, although Grok Build is earlier in the menu

#### Scenario: Duplicate rank rejected
- Given an injected list in which two integrations declare the same rank
- When the list is validated by test
- Then the validation fails

### Requirement: Declared follow policy (AIA-06)
Per-integration follow semantics that differ today MUST be declared per integration and preserved exactly, not unified. OpenCode MUST keep the bridge session channel (active session, view directory, connection epoch, unavailable and last-known roots) and stay the only owner that counts as agent-owned for the behavior that depends on it today. Claude Code MUST keep: an unusable directory is rejected with the detail `Claude Code directory is not usable`, leaving the root and its previous directory unchanged, and the tab retries that directory only at its next liveness check; while OpenCode holds the root, a reported directory is retained for when OpenCode releases it, and a release clears the retained directory without moving the root. Grok Build MUST keep: an unusable directory is ignored without a rejection and its previous directory stays; while OpenCode holds the root, Grok reports are ignored (nothing is retained or cleared); while Claude Code holds the root, a usable Grok directory is retained for when Claude Code exits, an unusable one is ignored and a release clears it. For every input and state, the update kind, the switched flag, the generation and the retained directories reported by follow MUST equal today's.

#### Scenario: Claude Code rejects an unusable directory
- Given Claude Code holds the root at directory A
- When it reports a missing directory B
- Then the update is rejected with `Claude Code directory is not usable`, the root stays A, and B is retried only at the next liveness check

#### Scenario: Grok ignores an unusable directory silently
- Given Grok holds the root at directory A
- When it reports a missing directory B
- Then the update is not a rejection, the root stays A, and Grok's previous directory stays A

#### Scenario: Claude Code retains under OpenCode
- Given OpenCode holds the root and Claude Code reports usable directory C
- When OpenCode disconnects
- Then the root becomes C

#### Scenario: Grok is ignored under OpenCode
- Given OpenCode holds the root, Grok previously held nothing, and Grok reports usable directory D
- When OpenCode disconnects
- Then the root returns to the shell, not to D, until Grok reports again

#### Scenario: Grok retains under Claude Code
- Given Claude Code holds the root and Grok reports usable directory D
- When Claude Code exits
- Then the root becomes D

#### Scenario: Branch table preserved
- Given the characterization table of every follow input (directory, release, unusable directory) against every holder state, recorded before the change
- When it runs after each delivery step
- Then every row yields the same update kind, switched flag, generation and retained directories

### Requirement: Declared launch policy (AIA-07)
Each integration MUST declare its direct-launch arguments and its launch-environment contribution: the variables it sets, the local targets that receive them (the shell target, its own direct target, or neither), and the variables removed from targets that do not bind them. The core MUST compose a tab's launch environment only from these declarations, for the first spawn and the fallback-directory retry alike. The result MUST equal today's for every target: a shell tab receives OpenCode's composed configuration and Made's Claude Code plugin entry and spool binding; a direct OpenCode tab receives the composed configuration and no Made Claude Code entry; a direct Grok tab runs `grok dashboard` with neither; a direct Claude Code tab receives the Claude Code entry and spool binding and no OpenCode configuration; `MADE_CLAUDE_SPOOL` is removed from every local target that does not bind a spool; remote targets receive no host integration environment and start no host process.

#### Scenario: Launch environment matrix
- Given the shell, direct OpenCode, direct Grok, direct Claude Code and remote targets, with and without a shipped Claude Code plugin and a spool
- When each target's launch is prepared, including the fallback retry
- Then program, arguments, variables set and variables removed equal the values recorded before the change

#### Scenario: Grok arguments
- Given Grok is on PATH
- When the user chooses Grok Build from the menu
- Then grok's absolute path starts with the single argument `dashboard`, and typing `grok` in a shell tab still adds no argument

### Requirement: Declared diagnostics contribution (AIA-08)
Each integration MUST declare its diagnostics contribution: whether it owns the primary integration status (OpenCode) or a secondary row, the row's state before detection, its copied-report keys, and its optional smoke-layout marker. Secondary rows, About rows, report lines and smoke markers MUST appear in list order. The declared values MUST reproduce today's literals: report keys `grok status`, `grok`, `claude status`, `claude`; pill labels `inactive`, `ready`, `degraded`; every row sentence, local and remote; smoke markers `opencode_plugin=` and `grok_status=` and no Claude Code marker.

#### Scenario: Rows before detection
- Given PATH detection has not finished
- When diagnostics are shown
- Then the Grok Build and Claude Code rows show the same pills and sentences as before the change

#### Scenario: Report order follows the list
- Given all three integrations
- When the report is copied
- Then the OpenCode status, reason code and reason lines come first, then the Grok Build lines, then the Claude Code lines, with unchanged keys

### Requirement: Set-pinning tests (AIA-09)
Tests that assert the set of integrations (the scope declaration, menu rows with every integration detected, detected-TUI lists, About rows, report lines, and the forbidden-surface scan of core sources) MUST derive their expectation of the set from the static list, so that a seam that stops iterating the list fails a test. Alongside them, explicit regression pins MUST keep asserting today's values verbatim: catalog order, programs (`opencode`, `grok`, `claude`), labels (`OpenCode`, `Grok Build`, `Claude Code`), menu ids (`opencode`, `grok`, `claude`), scope ids and names (`opencode-v2`/`OpenCode v2`, `grok-build`/`Grok Build`, `claude-code`/`Claude Code`), root-source identifiers and precedence order. The forbidden-surface scan MUST cover the core sources and every integration module after files move, and new identifiers MUST NOT contain a forbidden surface as a substring.

#### Scenario: A seam that ignores the list fails
- Given a seam is changed to skip one list entry
- When the committed derived tests run with the product list, the fake list or a list that lacks an entry
- Then at least one test fails

#### Scenario: A list change trips the pin
- Given an entry is added to, removed from or reordered in the product list
- When the tests run
- Then the explicit pin fails even though the derived tests pass

#### Scenario: Moved sources stay scanned
- Given integration code moved into integration modules
- When the scope-boundary test runs
- Then it scans those modules and the core for forbidden agent surfaces, and the fake integration's name is not a forbidden surface

### Requirement: Shared hook library (AIA-10)
Code that is identical in the native Grok hook and the native Claude Code hook MUST come from one shared library. Each hook MUST keep its own executable name, event-to-status mapping, record format, output target, limits and sanitization. For every input and environment, each hook's observable result (records written byte for byte given the same clock and pid ancestry, file location and permissions, exit status, empty stdout and stderr behavior, read deadline) MUST equal today's. Logic that differs between the hooks today, such as Claude Code's 8 MiB input cap and early completion gate, MUST keep each hook's current behavior. The external dependency set of each hook, the executable names, staging, packaging and the `Grok status hook` contract of `grok-native-hook` MUST NOT change.

#### Scenario: Golden corpus byte-identical
- Given a corpus of hook inputs (every event, malformed and truncated JSON, oversized input, split multibyte characters, slow and closed stdin) recorded with each hook's output before the change
- When each hook runs the corpus after the change with the same clock and ancestry
- Then every record, file and exit status is byte-identical

#### Scenario: Diverged read limits kept
- Given an input larger than 8 MiB
- When each hook reads it
- Then the Claude Code hook stops at its cap and the Grok hook behaves as it did before the change

#### Scenario: Packaged hooks unchanged
- Given the desktop package and companion stage sets of each target
- When they are built and their hook tests run from the packaged layout
- Then the same executables, names, modes and checksums-file entries are present and the packaged tests pass

### Requirement: Incremental delivery (AIA-11)
The change MUST be delivered as independently mergeable steps, in this order: (1) the common interface, the static list and generic identity, with every integration's runtime state owned by its module and the core iterating the list at every seam, plus Claude Code's channel code moved into its module; (2) Grok Build's remaining channel code moved into its module; (3) OpenCode's remaining channel code moved into its module and transitional paths removed; (4) the shared hook library; a final step MAY follow that carries only the scoped renames of misleading names and documentation, with no behavior change. After each step the full CI MUST be green and AIA-01 MUST hold, including while only some integrations' channel code is moved; within a step, every commit SHOULD build and pass the tests, and a group of commits that cannot be green until its last commit MUST be marked as such in `tasks.md`. Each step MUST be revertible without reverting the steps before it, and no step may persist data or formats that a revert would strand.

#### Scenario: Green after each step
- Given steps 1 to 4 merged one at a time
- When CI runs on each merge
- Then every workflow passes and AIA-01 holds at each point

#### Scenario: Revert of the latest step
- Given step N has merged after steps 1 to N-1
- When step N is reverted
- Then the build and CI are green and behavior is unchanged
