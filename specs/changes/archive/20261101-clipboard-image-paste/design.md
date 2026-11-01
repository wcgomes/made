# Design: clipboard-image-paste

## Approach

Made reads the system clipboard image through `made-platform` and routes the two paste chords in `made-app`. Native paste forwards `Ctrl+V` so the running TUI keeps ownership of clipboard formats and attachment behavior. The fallback writes a private PNG and pastes its absolute path, which agent TUIs can attach without a terminal image protocol. Text paste remains the existing bracketed-paste path, with image detection only in front of it.

## D1. Native-first paste with an explicit path fallback

Alternatives considered:

- **Native first, explicit fallback (chosen).** `Cmd+V`/`Ctrl+V` forwards `0x16`; `Cmd+Shift+V`/`Ctrl+Shift+V` saves a path. The child TUI decides how to read the image, so Made does not duplicate format conversion or attachment semantics.
- **Always save a path.** Works in more programs, but turns every image paste into a path even in TUIs that can attach the image directly, and changes the meaning of the standard paste chord.
- **Automatic fallback after a timeout.** Made cannot observe whether the child consumed `0x16` or ignored it, so a timeout would paste a second payload into programs that already handled the first one.
- **Forward `Ctrl+V` for every paste, including text.** This removes Made's bracketed-paste and sanitization behavior and would regress text paste.

The chosen order keeps the native path fast and expected, while the explicit fallback gives a deterministic path when the child has no clipboard reader. The trade-off is that the fallback is a second chord the user must know; the one-time notice closes that discovery gap.

## D2. Path fallback instead of image bytes or terminal protocols

There is no standard inbound image-clipboard channel for terminal applications:

- OSC 52 carries text only.
- OSC 5522 is kitty-specific and not widely implemented.
- The kitty graphics protocol, sixel, and iTerm2's `OSC 1337;File=` are one-way screen-output protocols; they cannot deliver clipboard bytes into a TUI.
- Base64 or raw image bytes pasted as text would explode the text-paste limit and require every child to decode an ad-hoc payload.

An absolute file path is compact, survives bracketed paste, and is already understood by OpenCode's prompt parser, which treats a pasted local path as an attachment, and by Claude Code, which accepts image paths. The path is intentionally unquoted: OpenCode strips surrounding quotes, but other TUIs may not, and a quoted path could be pasted literally in a shell. Paths containing spaces are therefore a residual limitation for TUIs that do not parse unquoted paths.

## D3. Image priority over text

A screenshot copied from a browser or chat often puts both an image and a URL or alt text on the clipboard. Text-first behavior would paste the URL and silently lose the image the user intended. Image-first matches the user's mental model and makes the fallback consistent: both chords treat the image as the payload.

## D4. Private temporary PNG

Clipboard images can contain passwords, private messages, or other on-screen secrets. The fallback therefore never writes into the workspace and never asks the user to choose a location. It uses the existing per-user `AppPaths::temp_dir`, creates `clipboard-images/` with mode `0700` on Unix, and creates each file with `create_new`, mode `0600`, and `O_NOFOLLOW`. The unique name combines the process id, a nanosecond timestamp, and an atomic counter so concurrent pastes cannot collide; a collision is retried rather than overwritten.

The directory is created and revalidated before and after creation, reusing the app's private-temp path checks. On Windows, `create_new` prevents overwrite, but a reparse-point swap remains part of the existing private-temp residual rather than a new mitigation claim.

Saved files are not kept indefinitely. The app-generated PNGs are pruned opportunistically after each successful save and when a workbench starts, using the fixed retention policy in D7. The operating system's temporary directory remains the outer lifetime boundary if Made is never started or never saves another image.

## D5. Bounded image size

The clipboard image is validated at 128 MiB of RGBA data and 16384 pixels per side. This covers 8K screenshots (7680x4320 RGBA is about 127 MiB) and typical high-resolution displays while bounding PNG encoding and per-paste memory. Images above the cap are refused by the path fallback; native paste may still forward `Ctrl+V` because the child may have a more efficient reader.

The cap is enforced after arboard has decoded the platform clipboard into RGBA. A hostile or unusual clipboard can therefore still allocate inside the platform library before Made validates it. The cap protects Made's encoding and file path, not the upstream decoder; that residual is accepted for a local desktop clipboard.

## D6. Generic native forwarding without agent detection

Made owns the PTY, not the foreground process tree, and cannot reliably identify whether the running program is OpenCode, Claude Code, grok, a plain shell, or another TUI. Detecting process names would be platform-specific and fragile. `Ctrl+V` is the common image-paste key for the targeted agents, so native forwarding is generic, and the first image paste shows a one-time hint naming the path fallback. In a plain shell, `Ctrl+V` enters quoted-insert mode rather than attaching an image; the fallback path remains available for that case.

## D7. Fixed opportunistic retention

The retention policy is intentionally small and deterministic: delete app-generated PNGs older than 7 days, then keep at most 64 files, oldest first. It runs after a successful save and when a workbench starts, so active use and the next launch both bound the directory without a background task, timer, configuration, or user-visible setting. The just-saved file is the newest candidate and therefore survives its own save's cleanup.

Alternatives rejected:

- **Delete on app exit:** a resumed session or a draft that still references the path would break, and the agent may read the path after the terminal closes.
- **Delete the previous image on every paste:** parallel panes and drafts can reference earlier paths, and a TUI may read lazily at submit time.
- **Only age-based cleanup:** a burst of pastes inside 7 days can still grow without bound.
- **Only count-based cleanup:** a dormant app can retain old sensitive images indefinitely.
- **Total-size cap:** useful, but adds a second accounting dimension; the count cap bounds the file count while the size cap would bound bytes. The fixed count is simpler and enough for the reported concern.
- **Background daemon or configurable retention:** unnecessary for a local desktop app and adds lifecycle and UI surface.

The pruner only considers regular files with the `clipboard-` prefix and `.png` suffix inside the app-owned `clipboard-images/` directory. It validates that directory first, skips symlinks and unrelated entries, ignores deletion errors, and returns the number removed. A file that survives age and count limits remains until the next save or workbench start.

## Flow

```
Cmd/Ctrl+V
  -> route_key -> native mode
  -> clipboard.get_image()
       Some(image) -> write 0x16 -> child reads clipboard; show one-time hint
       None        -> clipboard.get_text() -> existing write_paste(text)

Cmd/Ctrl+Shift+V
  -> route_key -> path mode
  -> clipboard.get_image()
       Some(image) -> PNG encode -> private save -> prune old/excess PNGs -> write_paste(absolute path)
       None        -> clipboard.get_text() -> existing write_paste(text)
```

Workbench start also runs the same prune before any new paste.

Both modes fall back to readable text when the image read errors, and both refuse to write a payload when neither read yields content.

## Architectural boundaries

- `made-platform`: `Clipboard` trait and image type, arboard-backed system clipboard, PNG encoding, private file save, `AppPaths::temp_dir`. OS-specific behavior stays here.
- `made-app` `terminal_view`: key routing, native forward, path fallback, notices, and writes into the existing PTY input path. It owns no OS clipboard code.
- `made-terminal`: unchanged; bracketed paste, text limits, and input encoding remain the shared text path.
- Bridge, explorer, renderer, session follow, and plugin code are untouched.

## Platform behavior

- **macOS:** arboard reads the pasteboard's TIFF representation and converts it to RGBA. `Cmd+V` is handled by Made before the child; `Cmd+Shift+V` is the fallback. The native runtime path has not been exercised in this change's automated environment.
- **Linux X11/Wayland:** arboard selects X11 or the Wayland data-control path. `Ctrl+V` and `Ctrl+Shift+V` are the chords. CI has no native compositor evidence for the clipboard path.
- **Windows:** arboard reads the clipboard image and converts it to RGBA. Made is the terminal, so Windows Terminal does not intercept the chord; the Windows runtime path is cross-compiled but not natively exercised.

## Migration and rollback

There is no schema, persisted state, configuration, or protocol migration. Fallback files exist only under the OS temporary directory and can be deleted safely. Rolling back removes the two key routes, the native forward, the path fallback, and the `Clipboard::get_image` extension; no stored data needs conversion. Files created before rollback remain until the operating system clears its temporary directory.

## Security and privacy considerations

- Clipboard content is untrusted input. It is never rendered, executed, or logged; only dimensions and byte length are exposed through `Debug`.
- The fallback path is app-generated, absolute, and UTF-8. A non-UTF-8 temp path is refused rather than pasted lossily.
- Bracketed-paste hardening still applies to the pasted path, although the path is app-generated.
- `0x16` forwarding is not command injection: the child interprets the byte through its own keymap. The consequence in a plain shell is quoted-insert, documented as a limitation.
- Private permissions, `create_new`, and `O_NOFOLLOW` reduce symlink and overwrite risk; the same-user TOCTOU and Windows reparse residuals already documented for app temp paths apply here.
- Retention only deletes regular files with the app-generated `clipboard-*.png` name inside the validated private directory. Symlink entries and unrelated files are skipped, and deletion errors are ignored so a cleanup failure cannot break the paste.

## Limitations and residual risks

- End-to-end image attachment inside OpenCode, Claude Code, or grok is not automated. The automated evidence covers the forwarded byte, the saved PNG, the pasted path, and the platform validation; native OS clipboard and real-TUI attachment remain pending.
- The path fallback depends on the child accepting a local image path. A plain shell receives a path, not an attached image.
- Unquoted paths with spaces may not attach in TUIs that do not parse them the way OpenCode does.
- Middle-click or primary-selection paste, drag-and-drop, and image paste from the explorer are out of scope.
- Retention is opportunistic: if Made is never started or never saves another image, old files remain until the operating system clears its temporary directory. A draft that references a path for more than 7 days can see it removed by the next save or workbench start.
- Saved PNGs are not individually pinned or user-managed; there is no UI to inspect or delete them.
