//! Claude Code spool lifecycle tests: per-instance directories, permissions,
//! tab-close deletion, startup cleanup of dead owners only, and truncation.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use made_app::claude_follow::{SpoolInstance, SpoolReader, MAX_RECORD_BYTES, SPOOL_BOUND};

fn line(session: &str) -> String {
    format!(
        "{{\"at\":\"t\",\"sessionId\":\"{session}\",\"cwd\":\"/w\",\"event\":\"Stop\",\"notification\":\"\",\"status\":\"idle\",\"title\":\"\",\"pids\":[1]}}\n"
    )
}

fn append(path: &Path, text: &str) {
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(text.as_bytes()).unwrap();
}

fn private_temp() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    }
    temp
}

/// Fill the spool the way the hook can: it appends only while the file stays
/// at or below the bound, so the file ends within one maximum record of it.
fn fill_to_the_bound(spool: &Path) -> usize {
    let mut written = fs::metadata(spool).unwrap().len();
    let mut count = 0;
    loop {
        let text = line(&format!("s{count}"));
        if written + text.len() as u64 > SPOOL_BOUND {
            break;
        }
        written += text.len() as u64;
        append(spool, &text);
        count += 1;
    }
    assert!(written <= SPOOL_BOUND);
    assert!(
        written + MAX_RECORD_BYTES as u64 > SPOOL_BOUND,
        "a further maximum-size record would be dropped by the hook"
    );
    count
}

#[cfg(unix)]
#[test]
fn spool_directory_and_file_are_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let temp = private_temp();
    let instance = SpoolInstance::create(temp.path()).unwrap();
    let spool = instance.create_spool("tab-1").unwrap();
    let dir_mode = fs::metadata(instance.dir()).unwrap().permissions().mode() & 0o777;
    let file_mode = fs::metadata(&spool).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o700);
    assert_eq!(file_mode, 0o600);
    assert!(spool.is_absolute());
    assert!(spool.starts_with(temp.path().join("claude")));
    let name = instance
        .dir()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(
        name.starts_with(&format!("{}-", std::process::id())),
        "the directory names its owner pid: {name}"
    );
    assert!(
        instance.create_spool("tab-1").is_err(),
        "an existing spool is never reused or followed"
    );
}

#[test]
fn closing_a_tab_deletes_only_its_spool() {
    let temp = private_temp();
    let instance = SpoolInstance::create(temp.path()).unwrap();
    let first = instance.create_spool("tab-1").unwrap();
    let second = instance.create_spool("tab-2").unwrap();
    instance.remove_spool("tab-1");
    assert!(!first.exists());
    assert!(second.exists());
    instance.remove_spool("tab-1");
    instance.remove_spool("never-created");
    assert!(second.exists());
}

#[test]
fn startup_removes_only_spools_of_instances_that_are_gone() {
    let temp = private_temp();
    let dead = SpoolInstance::create_for(temp.path(), 4_000_001, "dead").unwrap();
    let leftover = dead.create_spool("tab-1").unwrap();
    append(&leftover, &line("stale"));
    let dead_dir = dead.dir().to_path_buf();
    drop(dead);

    // A concurrently running second instance under a pid that is not ours.
    let second = SpoolInstance::create_for(temp.path(), 4_000_002, "second").unwrap();
    let second_spool = second.create_spool("tab-9").unwrap();
    append(&second_spool, &line("theirs"));

    let ours = SpoolInstance::create(temp.path()).unwrap();
    let ours_spool = ours.create_spool("tab-1").unwrap();
    let mut reader = SpoolReader::new(&ours_spool);
    let removed = ours.remove_stale();

    assert_eq!(removed, 1);
    assert!(!dead_dir.exists(), "the exited run's spools are removed");
    assert!(
        second_spool.exists(),
        "a running instance's spool is left intact"
    );
    assert_eq!(fs::read_to_string(&second_spool).unwrap(), line("theirs"));
    assert!(ours_spool.exists());
    assert!(
        reader.poll().is_empty(),
        "nothing from a leftover or another instance's spool is applied"
    );

    // Once the second instance exits, the next startup removes it too.
    let second_dir = second.dir().to_path_buf();
    drop(second);
    assert_eq!(ours.remove_stale(), 1);
    assert!(!second_dir.exists());
    assert!(ours.dir().exists());
}

#[test]
fn startup_leaves_unrelated_entries_alone() {
    let temp = private_temp();
    let claude = temp.path().join("claude");
    fs::create_dir_all(claude.join("not-an-instance")).unwrap();
    fs::write(claude.join("stray-file"), "x").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let ours = SpoolInstance::create(temp.path()).unwrap();
    assert_eq!(ours.remove_stale(), 0);
    assert!(claude.join("not-an-instance").exists());
    assert!(claude.join("stray-file").exists());
}

#[test]
fn a_fully_consumed_spool_near_the_bound_is_truncated_and_later_records_apply() {
    let temp = private_temp();
    let instance = SpoolInstance::create(temp.path()).unwrap();
    let spool = instance.create_spool("tab-1").unwrap();
    let mut reader = SpoolReader::new(&spool);

    let count = fill_to_the_bound(&spool);
    let records = reader.poll();
    assert_eq!(
        records.len(),
        count,
        "every record is applied before truncation"
    );
    assert_eq!(
        fs::metadata(&spool).unwrap().len(),
        0,
        "a fully consumed spool within one record of the bound is truncated"
    );

    append(&spool, &line("after-truncation"));
    let later = reader.poll();
    assert_eq!(later.len(), 1, "later records still apply");
    assert_eq!(later[0].session_id, "after-truncation");
}

#[test]
fn an_unconsumed_spool_near_the_bound_is_not_truncated() {
    let temp = private_temp();
    let instance = SpoolInstance::create(temp.path()).unwrap();
    let spool = instance.create_spool("tab-1").unwrap();
    let mut reader = SpoolReader::new(&spool);
    fill_to_the_bound(&spool);
    // A trailing partial record is not consumed, so the file is not fully consumed.
    append(&spool, "{\"sessionId\":\"par");
    let before = fs::metadata(&spool).unwrap().len();
    let _ = reader.poll();
    assert_eq!(fs::metadata(&spool).unwrap().len(), before);
}

#[test]
fn a_fully_consumed_small_spool_is_left_alone() {
    let temp = private_temp();
    let instance = SpoolInstance::create(temp.path()).unwrap();
    let spool = instance.create_spool("tab-1").unwrap();
    let mut reader = SpoolReader::new(&spool);
    append(&spool, &line("a"));
    assert_eq!(reader.poll().len(), 1);
    assert_eq!(fs::metadata(&spool).unwrap().len(), line("a").len() as u64);
}

#[cfg(unix)]
#[test]
fn truncation_waits_for_the_hook_lock() {
    let temp = private_temp();
    let instance = SpoolInstance::create(temp.path()).unwrap();
    let spool = instance.create_spool("tab-1").unwrap();
    let mut reader = SpoolReader::new(&spool);
    fill_to_the_bound(&spool);
    // The hook holds the advisory lock while it appends.
    let hook = fs::OpenOptions::new().append(true).open(&spool).unwrap();
    hook.lock().unwrap();
    let before = fs::metadata(&spool).unwrap().len();
    let records = reader.poll();
    assert!(!records.is_empty());
    assert_eq!(
        fs::metadata(&spool).unwrap().len(),
        before,
        "the reader must not truncate under the hook's lock"
    );
    drop(hook);
    assert!(reader.poll().is_empty());
    assert_eq!(
        fs::metadata(&spool).unwrap().len(),
        0,
        "truncated on a later poll"
    );
    append(&spool, &line("fresh"));
    assert_eq!(reader.poll().len(), 1);
}

// ---- the hook's path contract --------------------------------------------------

#[cfg(unix)]
#[test]
fn a_handed_out_spool_has_the_shape_the_hook_requires() {
    use std::os::unix::fs::PermissionsExt;
    let temp = private_temp();
    let instance = SpoolInstance::create(temp.path()).unwrap();
    let spool = instance.create_spool("tab-1").unwrap();
    let directory = spool.parent().unwrap();
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;

    assert!(spool.is_absolute());
    assert_eq!(spool.extension().and_then(|e| e.to_str()), Some("jsonl"));
    assert_eq!(directory.parent().unwrap().file_name().unwrap(), "claude");
    let name = directory.file_name().unwrap().to_str().unwrap();
    let (pid, id) = name.split_once('-').expect("<pid>-<id>");
    assert!(
        !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()),
        "{name}"
    );
    assert!(!id.is_empty(), "{name}");
    let lock = directory.join("owner.lock");
    assert!(
        fs::symlink_metadata(&lock).unwrap().is_file(),
        "owner.lock is a regular file beside the spool"
    );
    assert_eq!(mode(&lock), 0o600);
    assert!(fs::symlink_metadata(&spool).unwrap().is_file());
    assert_eq!(
        mode(&spool),
        0o600,
        "the file exists before any child starts"
    );
    assert_eq!(mode(directory), 0o700);
}

// ---- starting instances ----------------------------------------------------------

#[cfg(unix)]
#[test]
fn a_visible_owner_lock_is_always_already_held() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let temp = private_temp();
    let claude = temp.path().join("claude");
    let done = Arc::new(AtomicBool::new(false));
    let observer = {
        let claude = claude.clone();
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            let mut free = Vec::new();
            while !done.load(Ordering::Acquire) {
                let Ok(entries) = fs::read_dir(&claude) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let lock = entry.path().join("owner.lock");
                    let Ok(file) = fs::File::open(&lock) else {
                        continue;
                    };
                    if file.try_lock().is_ok() {
                        free.push(lock);
                    }
                }
            }
            free
        })
    };
    let mut kept = Vec::new();
    for index in 0..400 {
        // This process's pid is alive, so only the lock can protect the directory.
        kept.push(
            SpoolInstance::create_for(temp.path(), std::process::id(), &format!("race{index}"))
                .unwrap(),
        );
    }
    done.store(true, Ordering::Release);
    let free = observer.join().unwrap();
    assert!(
        free.is_empty(),
        "an owner.lock was visible before it was held: {free:?}"
    );
    let cleaner = SpoolInstance::create_for(temp.path(), 4_000_020, "cleaner").unwrap();
    assert_eq!(
        cleaner.remove_stale(),
        0,
        "running instances are not removed"
    );
    assert!(kept.iter().all(|instance| instance.dir().is_dir()));
}

// ---- the built hook, end to end ----------------------------------------------------

/// The staged hook (`MADE_PACKAGED_HOOK`) or the one `cargo build -p
/// made-claude-hook` leaves beside the test profile directory.
fn built_hook() -> Option<PathBuf> {
    if let Some(hook) = std::env::var_os("MADE_PACKAGED_HOOK").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(hook));
    }
    let profile = std::env::current_exe()
        .ok()?
        .parent()?
        .parent()?
        .to_path_buf();
    let hook = profile.join(format!(
        "made-claude-status{}",
        std::env::consts::EXE_SUFFIX
    ));
    hook.is_file().then_some(hook)
}

fn run_hook(hook: &Path, spool: &Path, session: &str) {
    let mut child = Command::new(hook)
        .env_clear()
        .env("MADE_CLAUDE_SPOOL", spool)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the hook");
    let payload = format!(
        "{{\"hook_event_name\":\"UserPromptSubmit\",\"session_id\":\"{session}\",\"cwd\":\"/work\"}}"
    );
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
}

#[cfg(unix)]
#[test]
fn the_built_hook_keeps_reporting_after_it_dropped_a_record_at_the_bound() {
    let Some(hook) = built_hook() else {
        eprintln!(
            "skipped: build the hook with `cargo build -p made-claude-hook` or set MADE_PACKAGED_HOOK"
        );
        return;
    };
    let temp = private_temp();
    let instance = SpoolInstance::create(temp.path()).unwrap();
    let spool = instance.create_spool("tab-1").unwrap();
    fill_to_the_bound(&spool);
    // Whatever the file holds when the tab starts is never applied.
    let mut reader = SpoolReader::new(&spool);

    // Drive the real hook until the bound makes it drop a record.
    let mut written = 0;
    let mut previous = fs::metadata(&spool).unwrap().len();
    let dropped = loop {
        run_hook(&hook, &spool, &format!("before{written}"));
        let now = fs::metadata(&spool).unwrap().len();
        if now == previous {
            break written;
        }
        assert!(now <= SPOOL_BOUND, "the hook never writes past the bound");
        previous = now;
        written += 1;
        assert!(written < 100, "the hook never dropped a record");
    };

    // The reader consumes what fit and, being within one record of the bound,
    // truncates, so the hook has room again.
    let records = reader.poll();
    assert_eq!(records.len(), dropped);
    assert_eq!(fs::metadata(&spool).unwrap().len(), 0);

    run_hook(&hook, &spool, "after");
    let later = reader.poll();
    assert_eq!(later.len(), 1, "a record after the drop is applied");
    assert_eq!(later[0].session_id, "after");
}

#[cfg(unix)]
#[test]
fn the_reader_never_follows_a_symlink_or_opens_a_fifo() {
    let temp = private_temp();
    let target = temp.path().join("victim.txt");
    let mut filler = line("victim");
    while (filler.len() as u64) + (MAX_RECORD_BYTES as u64) <= SPOOL_BOUND {
        filler.push_str(&line("victim"));
    }
    fs::write(&target, &filler).unwrap();
    let link = temp.path().join("link.jsonl");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let mut reader = SpoolReader::new(&link);
    assert!(reader.poll().is_empty(), "a symlinked spool is not read");
    assert_eq!(
        fs::metadata(&target).unwrap().len(),
        filler.len() as u64,
        "and never truncated"
    );

    let fifo = temp.path().join("fifo.jsonl");
    assert!(Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    let mut reader = SpoolReader::new(&fifo);
    assert!(reader.poll().is_empty(), "a FIFO neither blocks nor reads");
}
