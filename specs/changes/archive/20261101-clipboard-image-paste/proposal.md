# Proposal: Clipboard Image Paste

## Intent

Pasting an image from the system clipboard into Made's terminal currently fails: the terminal reads only text, so an image-only clipboard produces an error and the agent TUI never receives the image. Made must support image paste for OpenCode, Claude Code, grok, and similar terminal agents without replacing their native clipboard handling.

## Scope

In scope:

- Image detection on the system clipboard behind the `made-platform` clipboard abstraction.
- Native-first paste: `Cmd+V`/`Ctrl+V` forwards `Ctrl+V` to the child process when the clipboard holds an image.
- Explicit fallback: `Cmd+Shift+V`/`Ctrl+Shift+V` saves the image as a PNG in the app's private temporary directory and pastes its absolute path.
- Text-only paste, bracketed paste, copy, selection, and the existing 1 MiB text-paste limit remain unchanged.
- Image priority when the clipboard contains both text and an image.
- Clear notices for empty, unreadable, and oversized clipboards.
- Bounded retention for saved clipboard PNGs: prune app-generated files older than 7 days and keep at most 64, on save and workbench start.
- Resource and private-file controls for clipboard images.
- Automated regression coverage and descriptive documentation.

Non-goals:

- Inline image rendering, OCR, annotation, image editing, clipboard history, or writing images back to the clipboard.
- New terminal protocols for inbound image bytes such as OSC 52, OSC 5522, the kitty graphics protocol, or sixel.
- Detecting whether the foreground program accepts native image paste; the path fallback is an explicit user choice.
- A configurable retention policy, retention settings, or a background cleanup daemon.
- Formal integrations for specific agents; native forwarding is generic.
- Changing the existing 1 MiB text-paste limit, paste sanitization, or bracketed-paste marker hardening.
- Native OS clipboard runtime evidence on macOS, Linux, and Windows; that evidence remains pending.

## Approach

Add `Clipboard::get_image` to `made-platform` with an `Ok(None)` default so existing text-only implementors keep compiling, and implement it with arboard's `image-data` feature. Validate the returned RGBA dimensions and byte length, and keep image bytes out of `Debug` and logs. In `made-app`, `route_key` distinguishes the default paste from the path fallback. The default chord checks the image first: when one is present it forwards `0x16` and does not write a text payload; otherwise it preserves the existing text paste. The fallback chord encodes the image as PNG, saves it under `AppPaths::temp_dir/clipboard-images/`, and pastes the absolute path through the existing bracketed-paste path. The first native image paste per pane shows a one-time notice naming the fallback chord. The platform save helper bounds raw RGBA to 128 MiB and 16384 px per side, creates the directory `0700` and the file `0600` on Unix, uses `create_new` and `O_NOFOLLOW`, never overwrites, and refuses a non-UTF-8 path instead of pasting it lossily. App-generated clipboard PNGs are pruned after a successful save and when a workbench starts: files older than 7 days are removed, then at most 64 remain, oldest first, while the just-saved image is preserved. Expanded rationale, alternatives, platform behavior, and residual risks are in `design.md`.

## Decision index

- **Native-first paste with an explicit path fallback:** Made cannot observe whether the child consumed a forwarded `Ctrl+V`, so an automatic fallback could double-paste; the user selects the fallback chord deliberately (design: [D1](design.md#d1-native-first-paste-with-an-explicit-path-fallback)).
- **Paste an absolute path rather than base64 or a terminal image protocol:** no standard inbound image-clipboard channel exists, and agent TUIs such as OpenCode attach local image paths and Claude Code accepts them (design: [D2](design.md#d2-path-fallback-instead-of-image-bytes-or-terminal-protocols)).
- **Image wins over text:** a copied screenshot often also carries a URL or alt text, and the user's intent is the image (design: [D3](design.md#d3-image-priority-over-text)).
- **Private temporary PNG:** clipboard images can contain sensitive screen content, so they never enter the workspace and are stored with owner-only permissions and no-overwrite creation (design: [D4](design.md#d4-private-temporary-png)).
- **Bounded image size:** 128 MiB RGBA and 16384 px per side cover 8K screenshots while bounding memory; oversized images can still work natively but not through the path fallback (design: [D5](design.md#d5-bounded-image-size)).
- **Generic native forwarding without agent detection:** Made cannot reliably identify the foreground TUI from the PTY, and `Ctrl+V` is the common paste key for the targeted agents; a one-time hint exposes the fallback (design: [D6](design.md#d6-generic-native-forwarding-without-agent-detection)).
- **Fixed opportunistic retention:** prune app-generated PNGs older than 7 days and keep at most 64 files on save and workbench start; no background task or configuration (design: [D7](design.md#d7-fixed-opportunistic-retention)).
