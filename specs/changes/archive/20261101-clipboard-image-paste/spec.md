# Change Spec: clipboard-image-paste

## ADDED Requirements

### Requirement: Clipboard image paste

When the system clipboard contains an image, Made MUST treat the image as the paste payload for both paste chords, even when the clipboard also contains text. The behavior MUST be available on macOS, Linux on X11 and Wayland, and Windows.

The default paste chord — `Cmd+V` on macOS and `Ctrl+V` elsewhere — MUST forward the `Ctrl+V` control byte (`0x16`) to the child process and MUST NOT paste image bytes, clipboard text, or a file path. The child program is responsible for reading the image from the system clipboard. When the clipboard contains no image, the default chord MUST keep the existing text behavior: UTF-8 text is pasted as bracketed paste when the child enabled bracketed paste, subject to the existing 1 MiB text limit and sanitization.

The fallback chord — `Cmd+Shift+V` on macOS and `Ctrl+Shift+V` elsewhere — MUST encode the clipboard image as PNG, save it in the app's private temporary directory under `clipboard-images/`, and paste the absolute file path as text through the existing bracketed-paste path. The fallback MUST NOT write the image into the workspace, overwrite an existing file, or follow a symlink. When the clipboard contains no image, the fallback chord MUST continue to paste text exactly like the default chord.

The first successful native image paste in a terminal pane MUST show a notice that names the fallback chord and MUST NOT read as an error. Later native image pastes in the same pane MUST NOT repeat that notice. The notice requirement applies again after the terminal is relaunched in that pane.

If the clipboard is empty or neither an image nor text can be read, Made MUST show a clear notice and MUST NOT paste text or write an image file. In native mode, when the image read fails and no text is readable, Made MAY forward Ctrl+V without an accompanying image payload; the same notice requirement applies. In path fallback mode, no payload is written.

An image larger than 128 MiB of RGBA data or larger than 16384 pixels on either side MUST NOT be encoded or saved by the path fallback; the fallback MUST report that nothing was pasted. Native mode MAY still forward Ctrl+V when the image read is rejected by these limits.

Clipboard image bytes and clipboard text MUST NOT appear in logs or user-facing notices. `Debug` output for a clipboard image MUST report dimensions and byte length only. On Unix, the saved PNG directory MUST be mode `0700` and each saved file MUST be mode `0600`; files MUST be created without overwriting an existing path and without following a symlink. The path handed to the application MUST be absolute and valid UTF-8; a non-UTF-8 path MUST be refused instead of pasted lossily.

#### Scenario: Native image paste in an agent TUI

- Given an agent TUI is running in the terminal and the clipboard contains only an image
- When the user presses Cmd+V on macOS or Ctrl+V elsewhere
- Then Made forwards the Ctrl+V control byte to the TUI, does not paste text, image bytes, or a path, and the TUI can read the clipboard image

#### Scenario: Native image paste hint

- Given the clipboard contains an image and the pane has not yet shown the native-paste hint
- When the user performs a native image paste, then performs another native image paste
- Then the first paste shows a notice naming the fallback chord, the second does not repeat it, and relaunching the terminal allows the notice once again

#### Scenario: Path fallback

- Given the clipboard contains an image
- When the user presses Cmd+Shift+V on macOS or Ctrl+Shift+V elsewhere
- Then a PNG with a valid signature is saved under the app's private temporary directory, its absolute path is pasted as bracketed text, and no workspace file is written

#### Scenario: Text-only paste remains unchanged

- Given the clipboard contains only text
- When the user presses either paste chord
- Then the text is pasted with the existing bracketed-paste behavior, subject to the existing limit and sanitization, and no image file is created

#### Scenario: Image wins over text

- Given the clipboard contains both text and an image
- When the user presses either paste chord
- Then the image is treated as the payload and the text is not pasted

#### Scenario: Empty or unreadable clipboard

- Given the clipboard is empty or both image and text reads fail
- When the user presses either paste chord
- Then Made shows a clear notice, does not panic, and pastes no text or image file; native mode may forward Ctrl+V as a last resort

#### Scenario: Oversized image

- Given the clipboard image exceeds the RGBA byte cap or the per-side pixel cap
- When the user presses the fallback chord
- Then Made refuses to encode or save it, shows that nothing was pasted, and does not create a file; a native paste may still forward Ctrl+V

#### Scenario: Private image storage

- Given the path fallback saves an image on Unix
- When the file and its directory are inspected
- Then the file is `0600`, the `clipboard-images` directory is `0700`, the file did not overwrite an existing path, and no image bytes or clipboard text appear in logs or Debug output

### Requirement: Clipboard image retention

Made MUST bound the app-generated PNGs kept under `clipboard-images/`. After each successful path-fallback save and when a workbench starts, Made MUST remove app-generated PNGs whose modification time is older than 7 days. After the age check, Made MUST keep at most 64 app-generated PNGs, removing the oldest first. The image saved by the current paste MUST survive that save's cleanup. Cleanup MUST consider only regular files whose names begin with `clipboard-` and end with `.png`; unrelated files and symlink entries MUST NOT be removed. Cleanup failures MUST NOT fail the paste, MUST NOT remove unrelated files, and MUST leave the saved path usable. Cleanup MUST NOT log image content and MUST NOT require a background task, configuration, or user-visible setting.

#### Scenario: Age-based cleanup

- Given `clipboard-images/` contains an app-generated PNG modified more than 7 days ago and a current app-generated PNG
- When a new image is saved through the path fallback
- Then the old PNG is removed, the current PNG remains, and the newly saved image exists

#### Scenario: Count cap

- Given `clipboard-images/` already contains 67 app-generated PNGs
- When a new image is saved through the path fallback
- Then at most 64 app-generated PNGs remain, the oldest ones were removed first, and the path returned for the new paste still exists

#### Scenario: Unrelated entries are preserved

- Given `clipboard-images/` contains an old app-generated PNG, an unrelated `other.png`, a `note.txt`, and a symlink whose name starts with `clipboard-`
- When cleanup runs
- Then only the old app-generated regular PNG is removed, and the unrelated file, text file, symlink, and its target remain

#### Scenario: Cleanup failure does not block the paste

- Given an app-generated PNG cannot be removed because the file or directory denies deletion
- When the path fallback saves a new image
- Then the new image is saved and its path remains usable, the cleanup error is not treated as a paste failure, and unrelated files remain untouched
