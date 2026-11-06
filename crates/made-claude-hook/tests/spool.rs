//! The spool contract: where the hook may write, with which permissions, and
//! how large a record and a file can get.
#![cfg(unix)]

mod common;

use std::io::Write;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use common::{assert_silent, event_json, run_closed, run_event, run_open, Spool};
use serde_json::json;

const MIB: u64 = 1024 * 1024;

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn stop() -> Vec<u8> {
    event_json("Stop", &[])
}

#[test]
fn creates_the_file_with_mode_0600_and_appends() {
    let spool = Spool::new();
    assert_eq!(run_event(&spool, &stop()).len(), 1);
    assert_eq!(mode(&spool.file), 0o600);
    assert_eq!(mode(&spool.dir), 0o700);
    assert_eq!(run_event(&spool, &stop()).len(), 2);
    assert_eq!(mode(&spool.file), 0o600);
}

#[test]
fn relative_spool_path_writes_nothing() {
    let spool = Spool::new();
    let mut command = Command::new(common::HOOK);
    command
        .env_clear()
        .env("MADE_CLAUDE_SPOOL", "claude/1-abc/ws-1.jsonl")
        .current_dir(spool.root.path())
        .stdin(std::process::Stdio::piped());
    let mut child = command.spawn().unwrap();
    child.stdin.take().unwrap().write_all(&stop()).unwrap();
    assert!(child.wait().unwrap().success());
    assert!(!spool.file.exists());
}

#[test]
fn missing_parent_directory_is_not_created() {
    let spool = Spool::new();
    let file = spool.root.path().join("absent/ws-1.jsonl");
    let outcome = run_closed(Some(&file), &stop());
    assert_silent(&outcome);
    assert!(!spool.root.path().join("absent").exists());
}

#[test]
fn non_private_parent_directory_writes_nothing() {
    for bad in [0o755, 0o770, 0o777, 0o500] {
        let spool = Spool::new();
        std::fs::set_permissions(&spool.dir, std::fs::Permissions::from_mode(bad)).unwrap();
        let outcome = run_closed(Some(&spool.file), &stop());
        assert_silent(&outcome);
        std::fs::set_permissions(&spool.dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(!spool.file.exists(), "mode {bad:o}");
    }
}

#[test]
fn symlinked_parent_directory_writes_nothing() {
    let spool = Spool::new();
    let link = spool.root.path().join("link");
    symlink(&spool.dir, &link).unwrap();
    let outcome = run_closed(Some(&link.join("ws-1.jsonl")), &stop());
    assert_silent(&outcome);
    assert!(!spool.file.exists());
}

#[test]
fn symlinked_spool_file_is_refused() {
    let spool = Spool::new();
    let target = spool.dir.join("target.txt");
    std::fs::write(&target, "").unwrap();
    symlink(&target, &spool.file).unwrap();
    let outcome = run_closed(Some(&spool.file), &stop());
    assert_silent(&outcome);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "");

    // A dangling link must not be created through either.
    std::fs::remove_file(&spool.file).unwrap();
    let missing = spool.dir.join("missing.txt");
    symlink(&missing, &spool.file).unwrap();
    assert_silent(&run_closed(Some(&spool.file), &stop()));
    assert!(!missing.exists());
}

#[test]
fn existing_file_that_is_not_private_is_refused() {
    let spool = Spool::new();
    std::fs::write(&spool.file, "").unwrap();
    std::fs::set_permissions(&spool.file, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_silent(&run_closed(Some(&spool.file), &stop()));
    assert_eq!(spool.raw(), "");
}

#[test]
fn record_over_4_kib_is_not_written() {
    let spool = Spool::new();
    let big = "d".repeat(4200);
    let records = run_event(&spool, &event_json("Stop", &[("cwd", json!(big))]));
    assert!(records.is_empty());

    // A record well inside the bound still goes through.
    let ok = "d".repeat(1000);
    let records = run_event(&spool, &event_json("Stop", &[("cwd", json!(ok))]));
    assert_eq!(records.len(), 1);
    assert!(spool.raw().len() <= 4096);
}

#[test]
fn full_file_drops_the_record_and_keeps_the_size() {
    let spool = Spool::new();
    let filler = vec![b'\n'; MIB as usize];
    std::fs::write(&spool.file, &filler).unwrap();
    std::fs::set_permissions(&spool.file, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_silent(&run_closed(Some(&spool.file), &stop()));
    assert_eq!(std::fs::metadata(&spool.file).unwrap().len(), MIB);
}

#[test]
fn file_never_grows_past_the_bound() {
    let spool = Spool::new();
    // Leave less room than one record needs.
    let filler = vec![b'\n'; (MIB - 20) as usize];
    std::fs::write(&spool.file, &filler).unwrap();
    std::fs::set_permissions(&spool.file, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_silent(&run_closed(Some(&spool.file), &stop()));
    assert_eq!(std::fs::metadata(&spool.file).unwrap().len(), MIB - 20);
}

#[test]
fn consumed_file_accepts_records_again() {
    let spool = Spool::new();
    let filler = vec![b'\n'; MIB as usize];
    std::fs::write(&spool.file, &filler).unwrap();
    std::fs::set_permissions(&spool.file, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_silent(&run_closed(Some(&spool.file), &stop()));
    // The reader truncates a consumed file; the next event is applied.
    std::fs::write(&spool.file, "").unwrap();
    assert_eq!(run_event(&spool, &stop()).len(), 1);
}

#[test]
fn lock_is_shared_with_the_reader_and_never_blocks_the_hook() {
    let spool = Spool::new();
    assert_eq!(run_event(&spool, &stop()).len(), 1);
    let reader = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&spool.file)
        .unwrap();
    reader.lock().unwrap();
    // A held lock means the hook drops the record, quickly.
    let outcome = run_open(Some(&spool.file), &stop(), Duration::from_secs(3));
    assert_silent(&outcome);
    assert!(
        outcome.elapsed < Duration::from_secs(2),
        "{:?}",
        outcome.elapsed
    );
    assert_eq!(spool.records().len(), 1);
    reader.unlock().unwrap();

    // The hook released its own lock on exit.
    assert_eq!(run_event(&spool, &stop()).len(), 2);
    assert!(reader.try_lock().is_ok());
}

#[test]
fn pid_chain_starts_at_the_parent_and_is_nearest_first() {
    let spool = Spool::new();
    let records = run_event(&spool, &stop());
    let pids: Vec<u64> = records[0]["pids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pid| pid.as_u64().unwrap())
        .collect();
    assert_eq!(pids[0], u64::from(std::process::id()));
    assert!(pids.len() <= 32);
    assert!(pids.iter().all(|pid| *pid > 0));
    // The test's own parent comes next when it is visible.
    let parent = u64::from(std::os::unix::process::parent_id());
    if parent > 0 {
        assert_eq!(pids.get(1), Some(&parent));
    }
}

#[test]
fn pid_chain_is_capped_at_32_entries() {
    let spool = Spool::new();
    let work = tempfile::tempdir().unwrap();
    let payload = work.path().join("payload.json");
    std::fs::write(&payload, stop()).unwrap();
    // 40 nested shells, each kept alive by a trailing command so none is
    // replaced by exec.
    let script = work.path().join("nest.sh");
    std::fs::write(
        &script,
        "if [ \"$1\" -gt 0 ]; then sh \"$0\" $(($1 - 1)); r=$?; : ; exit $r; fi\n\
         \"$HOOK\" < \"$PAYLOAD\"\n",
    )
    .unwrap();
    let status = Command::new("sh")
        .arg(&script)
        .arg("40")
        .env("HOOK", common::HOOK)
        .env("PAYLOAD", &payload)
        .env("MADE_CLAUDE_SPOOL", &spool.file)
        .status()
        .unwrap();
    assert!(status.success());
    let records = spool.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["pids"].as_array().unwrap().len(), 32);
}
