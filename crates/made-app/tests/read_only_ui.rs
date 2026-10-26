//! Read-only preview tests: no edit/save command exists, save shortcuts are
//! inert, loading/rendering never writes, and binary/large/invalid files only
//! produce safe messages.

use std::fs;
use std::io::Write;

use eframe::egui::{Key, Modifiers};
use made_app::explorer_view::{
    document_text, preview_shortcut_action, PreviewAction, PreviewDocument, PREVIEW_ACTIONS,
};
use made_explorer::highlight::Highlighter;
use made_explorer::preview::{PreviewLoader, PREVIEW_SIZE_LIMIT};

#[test]
fn the_preview_action_set_has_no_edit_or_save_command() {
    for action in PREVIEW_ACTIONS {
        let label = action.label().to_ascii_lowercase();
        assert!(
            !label.contains("save") && !label.contains("edit") && !label.contains("write"),
            "preview exposes a mutating action: {label}"
        );
    }
    assert_eq!(PREVIEW_ACTIONS.len(), 3);
    assert!(PREVIEW_ACTIONS.contains(&PreviewAction::Refresh));
    assert!(PREVIEW_ACTIONS.contains(&PreviewAction::CopyPath));
    assert!(PREVIEW_ACTIONS.contains(&PreviewAction::Close));
}

#[test]
fn ctrl_and_cmd_s_are_inert() {
    let ctrl = Modifiers {
        ctrl: true,
        command: true,
        ..Modifiers::NONE
    };
    let cmd = Modifiers {
        mac_cmd: true,
        command: true,
        ..Modifiers::NONE
    };
    let ctrl_shift = Modifiers {
        ctrl: true,
        command: true,
        shift: true,
        ..Modifiers::NONE
    };
    assert_eq!(preview_shortcut_action(Key::S, ctrl), None);
    assert_eq!(preview_shortcut_action(Key::S, cmd), None);
    assert_eq!(preview_shortcut_action(Key::S, ctrl_shift), None);

    assert_eq!(
        preview_shortcut_action(Key::F5, Modifiers::NONE),
        Some(PreviewAction::Refresh)
    );
    assert_eq!(
        preview_shortcut_action(
            Key::C,
            Modifiers {
                command: true,
                shift: true,
                ..Modifiers::NONE
            }
        ),
        Some(PreviewAction::CopyPath)
    );
}

#[test]
fn loading_and_rendering_a_text_file_never_modifies_it() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("main.rs");
    let original = "fn main() {\n    println!(\"hello\");\n}\n";
    fs::write(&file, original).unwrap();
    let before = fs::read(&file).unwrap();

    let loader = PreviewLoader::new(temp.path()).unwrap();
    let preview = loader.load(&file);
    let highlighter = Highlighter::new();
    let document = PreviewDocument::from_preview(preview, &highlighter);

    assert!(document.is_text());
    assert_eq!(document_text(&document), original.trim_end_matches('\n'));
    assert!(document.line_count() >= 3);
    assert_eq!(fs::read(&file).unwrap(), before, "preview wrote the file");
}

#[test]
fn binary_files_return_a_safe_message_and_no_content() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("image.bin");
    let mut bytes = vec![0_u8, 1, 2, 3, 0, 4, 5];
    fs::write(&file, &bytes).unwrap();
    bytes.clear();

    let loader = PreviewLoader::new(temp.path()).unwrap();
    let preview = loader.load(&file);
    let highlighter = Highlighter::new();
    let document = PreviewDocument::from_preview(preview, &highlighter);

    assert!(!document.is_text());
    assert!(document.lines.is_empty());
    assert!(
        document
            .message
            .as_deref()
            .is_some_and(|message| message.contains("Binary")),
        "unexpected binary message: {:?}",
        document.message
    );
}

#[test]
fn oversized_files_return_a_size_message_and_no_content() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("large.txt");
    let handle = fs::File::create(&file).unwrap();
    handle.set_len(PREVIEW_SIZE_LIMIT + 1024).unwrap();
    drop(handle);

    let loader = PreviewLoader::new(temp.path()).unwrap();
    let preview = loader.load(&file);
    assert!(!preview.is_text());
    let highlighter = Highlighter::new();
    let document = PreviewDocument::from_preview(preview, &highlighter);

    assert!(document.lines.is_empty());
    let message = document.message.expect("size message");
    assert!(
        message.contains("limit") || message.contains("limited"),
        "{message}"
    );
    assert!(message.contains(&PREVIEW_SIZE_LIMIT.to_string()));
}

#[test]
fn invalid_utf8_and_missing_files_are_safe() {
    let temp = tempfile::tempdir().unwrap();
    let invalid = temp.path().join("invalid.txt");
    let mut file = fs::File::create(&invalid).unwrap();
    file.write_all(&[0xff, 0xfe, 0xfd]).unwrap();
    drop(file);

    let loader = PreviewLoader::new(temp.path()).unwrap();
    let highlighter = Highlighter::new();
    let document = PreviewDocument::from_preview(loader.load(&invalid), &highlighter);
    assert!(!document.is_text());
    assert!(document.lines.is_empty());
    assert!(document
        .message
        .as_deref()
        .is_some_and(|message| message.contains("UTF-8")));

    let missing = temp.path().join("gone.txt");
    let document = PreviewDocument::from_preview(loader.load(&missing), &highlighter);
    assert!(!document.is_text());
    assert!(document.lines.is_empty());
    assert!(document
        .message
        .as_deref()
        .is_some_and(|message| message.contains("missing")));
}
