//! Unit tests for the items both native hooks share.

use std::process::Command;
use std::time::Duration;

use made_hook_common::{
    field_text, format_timestamp, json_string, run_silently, text_of, Chunk, POLL_SLICE,
    READ_DEADLINE,
};
use serde_json::{Map, Value};

fn object(text: &str) -> Map<String, Value> {
    serde_json::from_str(text).unwrap()
}

#[test]
fn deadline_and_poll_slice_keep_their_values() {
    assert_eq!(READ_DEADLINE, Duration::from_secs(2));
    assert_eq!(POLL_SLICE, Duration::from_millis(200));
}

#[test]
fn chunks_compare_by_content() {
    assert_eq!(Chunk::Data(b"a".to_vec()), Chunk::Data(b"a".to_vec()));
    assert_ne!(Chunk::Data(b"a".to_vec()), Chunk::Data(b"b".to_vec()));
    assert_ne!(Chunk::Eof, Chunk::Idle);
}

#[test]
fn timestamp_formats_utc_seconds() {
    assert_eq!(format_timestamp(0), "1970-01-01T00:00:00Z");
    assert_eq!(format_timestamp(951_782_400), "2000-02-29T00:00:00Z");
    assert_eq!(format_timestamp(1_767_225_599), "2025-12-31T23:59:59Z");
    assert_eq!(format_timestamp(4_102_444_800), "2100-01-01T00:00:00Z");
}

#[test]
fn json_string_escapes_and_quotes() {
    assert_eq!(json_string("plain"), "\"plain\"");
    assert_eq!(json_string("a\"b\\c\n"), "\"a\\\"b\\\\c\\n\"");
    assert_eq!(json_string("caf\u{e9}"), "\"caf\u{e9}\"");
}

#[test]
fn field_text_is_the_string_value_or_empty() {
    let event = object(r#"{"a":"x","b":1,"c":null,"d":""}"#);
    assert_eq!(field_text(&event, "a"), "x");
    assert_eq!(field_text(&event, "b"), "");
    assert_eq!(field_text(&event, "c"), "");
    assert_eq!(field_text(&event, "d"), "");
    assert_eq!(field_text(&event, "missing"), "");
}

#[test]
fn text_of_takes_the_first_non_empty_field() {
    let event = object(r#"{"a":"","b":"second","c":"third","n":5}"#);
    assert_eq!(text_of(&event, &["a", "b", "c"]), "second");
    assert_eq!(text_of(&event, &["n", "missing"]), "");
    assert_eq!(text_of(&event, &[]), "");
}

const CHILD_FLAG: &str = "MADE_HOOK_COMMON_SILENT_CHILD";

/// Entry point of the child process below; a no-op in a normal run.
#[test]
fn silent_child_entry() {
    if std::env::var_os(CHILD_FLAG).is_some() {
        run_silently(|| panic!("must stay silent"));
    }
}

#[test]
fn a_panicking_run_exits_zero_with_empty_standard_error() {
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "silent_child_entry", "--test-threads=1"])
        .env(CHILD_FLAG, "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
}
