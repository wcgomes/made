# Proposal: MADE wordmark About trigger and About screen

## Intent

The header currently spends one icon-only slot on Diagnostics, and the floating card opens straight into technical detail. This change removes the Diagnostics icon, makes the existing MADE wordmark the persistent About trigger, and turns the card into an About screen: the default view shows the MADE identity, the Made application name and version, and a short product description, with a labeled button that reveals the existing diagnostics detail on demand. The wordmark's brand identity, the header spacing, and the Diagnostics data and behavior remain unchanged.

## Scope

This is the active change `wordmark-diagnostics` against the realized `specs/made-ui-ux.md`. `specs/changes/` has no other active change; archived changes are not edited. The delta MODIFIES `Minimal Workspace Layout`, `Brand and Workspace Tabs`, `Keyboard Focus and Accessibility`, and `Diagnostics Presentation`, and ADDS `About Presentation`. It removes Diagnostics from the persistent icon-only header actions, orders the remaining right icon group Sessions (when shown) then Explorer with Explorer rightmost, and makes the MADE wordmark the persistent About trigger. While a workspace is active, pointer or keyboard activation opens and closes the About card; without an active workspace it does not. The wordmark keeps its two-tone MADE identity and paints no tab fill, hover fill, or selection treatment, gains a pointer cursor, an `About` tooltip and accessibility name, a target of at least 24×24, a visible focus ring, and Enter/Space activation. About defaults to the wordmark, the Made name and version, a short product description, and a `Show diagnostics` button; the status pill, diagnostics sections, log block, log paths, copy actions, and notices stay hidden until the button is activated, which reveals the existing Diagnostics content and changes the label to `Hide diagnostics`. Closing and reopening About returns to the default view. The card is fixed, centered, and non-resizable: it stays compact while diagnostics are hidden and grows smoothly to use the available application height when they are revealed. While diagnostics are open, the toggle uses the active accent treatment so it reads as the close action; while closed, it uses the secondary treatment. The reveal fades and grows over about 220 ms and reverses when hidden. The tab-strip-side reserved gap stays 24px wide / 18px compact and the wordmark-to-first-icon gap stays exactly 24px wide / 18px compact. Diagnostics data, redaction, retention, and log content are unchanged.

## Non-goals

- Changing Diagnostics data fields, status semantics, redaction, retention, log content, or the bounded log behavior.
- Adding a new Diagnostics surface, keyboard shortcut, status chrome, banner, or pill.
- Making the picker wordmark an About trigger, or changing the picker wordmark's paint or behavior.
- Adding a text label, hover fill, selected fill, tab treatment, divider, or second About control to the header.
- Adding external links, license text, dependency credits, update checks, or telemetry to About.
- Restoring or extending pointer resize or move affordances on the About card.
- Changing the header height, tab-strip gaps, Sessions behavior, Explorer behavior, or the Open folder action.
- Changing unrelated icon-only actions' tooltips, accessible names, targets, or focus treatment.
- Editing `specs/made-ui-ux.md`, archived changes, or the wiki as part of this change-authoring step.
- Creating `design.md` or archiving this change.

## Approach

Record the expanded behavior as a delta on the realized UI contract. The change remains small enough for the decision index, so `design.md` is not created. Implementation will keep the cached two-tone paint and the existing header layout slots, rename the header trigger from Diagnostics to About, remove the header icon path, and split the existing card into a default About block plus an on-demand diagnostics detail gated by one labeled toggle. The toggle state resets when the card is reopened. Tests will assert the visible outcomes before implementation. Implementation detail and paths stay in `tasks.md`; normative outcomes stay in `spec.md`.

Conflicts are flagged and not silently preferred. `wiki/ui/layout-and-controls.md` still describes the wordmark tooltip as "Minimal Agentic Development Environment" and orders the right icon group Sessions, Diagnostics, Explorer; `wiki/records/20261026-v1-evidence.md` and `wiki/testing/strategy.md` also record that old order. `wiki/ui/design-system.md` still records a 28px/20px compact wordmark-adjacent gap and a wordmark-before-tabs placement, which conflict with the realized 24px/18px gaps and the wordmark-before-icons placement. Those pages are descriptive, are not the contract, and are not edited here except for reviewed wording that this change makes stale. The live contract is `specs/made-ui-ux.md` plus this active delta.

## Decision index

- **Wordmark replaces the Diagnostics icon:** remove Diagnostics from the icon-only header actions; the right icon group orders Sessions (when shown) then Explorer, with Explorer rightmost.
- **Wordmark trigger scope:** the header MADE wordmark is the persistent About trigger; pointer or keyboard activation opens and closes the About card only while a workspace is active, and activation without an active workspace does not open or close it.
- **Brand identity retained:** the two-tone `M` accent and `ADE` primary wordmark, monospace tracking, cached paint, and no tab/fill/hover/selection treatment remain.
- **About metadata:** the header wordmark tooltip and accessibility name are exactly `About`; the former "Minimal Agentic Development Environment" tooltip is not retained on that trigger. The picker wordmark stays brand-only and keeps its existing presentation.
- **About default content:** the card shows the MADE wordmark, the Made application name and version, a short product description, and a `Show diagnostics` button. No status pill, diagnostics sections, log block, log paths, copy diagnostics actions, or notices are shown by default.
- **Diagnostics on demand:** activating `Show diagnostics` reveals the existing Diagnostics detail in the same card and changes the label to `Hide diagnostics`; the second activation hides it. The About block remains the card's identity.
- **Reset on reopen:** closing and reopening About returns to the default About-only view with diagnostics hidden.
- **Fixed, centered card:** About has no resize or move affordance and stays centered. It uses a compact height while diagnostics are hidden and grows to use the available app height when they are revealed; hiding or reopening returns it to the compact height.
- **Active toggle and animated reveal:** while diagnostics are open, the toggle uses the active accent treatment so it reads as the close action; when closed, it returns to the secondary treatment. The detail fades in and the card grows over about 220 ms and reverses when hidden.
- **Diagnostics presentation unchanged when revealed:** the status pill, reason, copy actions, sections, bounded log block, previous-log row, active-log row, retention tone, and redaction remain as before.
- **Interaction and accessibility:** pointer cursor, target at least 24×24, visible focus ring, and Enter/Space activation for the wordmark; the diagnostics toggle is keyboard reachable with a visible focus ring and matching accessible name.
- **Spacing unchanged:** the tab-strip-side reserved gap stays 24px wide / 18px compact and the wordmark-to-first-icon gap stays exactly 24px wide / 18px compact; the whole right cluster stays reserved so tabs, `+`, and overflow never overlap it.
- **No design overflow:** the index is sufficient, so `design.md` is omitted.
- **Live contract routing:** the delta targets `made-ui-ux`; the realized spec, archived changes, and the wiki are not edited until a later confirmed archive.

## Clarifications

- **Q:** Which control replaces the header Diagnostics icon?
  **A:** The persistent MADE wordmark. The icon-only header actions keep Open folder, Explorer, and the optional Sessions action; the right icon group orders Sessions then Explorer, with Explorer rightmost.
- **Q:** Does the wordmark keep its brand look?
  **A:** Yes. It keeps the two-tone MADE identity, monospace tracking, and no tab, fill, hover, or selection treatment. It gains a pointer cursor, a visible focus ring, and Enter/Space activation.
- **Q:** What tooltip and accessibility name does the wordmark use?
  **A:** Both are exactly `About`, matching the About screen. The former brand expansion tooltip is replaced on the header trigger; the picker wordmark remains brand-only.
- **Q:** When does activating the wordmark open About?
  **A:** Only while a workspace is active. Without an active workspace it does not open or close the card.
- **Q:** What does About show before diagnostics are revealed?
  **A:** The MADE wordmark, the Made application name and version, a short product description, and a `Show diagnostics` button. No technical or status detail is shown.
- **Q:** How are diagnostics revealed?
  **A:** A labeled `Show diagnostics` button reveals the existing Diagnostics detail in the same card and becomes `Hide diagnostics`; activating it again hides the detail. Closing and reopening About returns to the default About view.
- **Q:** Does the About card resize?
  **A:** No. The resize affordance is removed and the card is fixed and centered. It stays compact while diagnostics are hidden and grows to use the available application height when diagnostics are revealed; hiding them or reopening About returns it to the compact height.
- **Q:** How does the user know the diagnostics toggle closes the detail?
  **A:** While diagnostics are open, the toggle uses the active accent treatment with the `Hide diagnostics` label; when closed, it returns to the secondary treatment with `Show diagnostics`. The detail fades in and the card grows over about 220 ms; hiding reverses the animation to the compact About card.
- **Q:** Do the header gaps change?
  **A:** No. The tab-strip-side reserved gap stays 24px wide / 18px compact, and the wordmark-to-first-icon gap stays exactly 24px wide / 18px compact.
- **Q:** Is Diagnostics data or presentation changing when revealed?
  **A:** No. The status pill, reason, copy actions, sections, logs, retention messaging, and redaction are unchanged.
