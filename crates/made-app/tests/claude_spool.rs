//! Claude Code spool lifecycle tests: per-instance directories, permissions,
//! tab-close deletion, startup cleanup of dead owners only, and truncation.

use std::fs;
use std::io::Write;
use std::path::Path;

use made_app::claude_follow::{SpoolInstance, SpoolReader, SPOOL_BOUND};

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

fn fill_past_the_bound(spool: &Path) -> usize {
    let mut written = 0_u64;
    let mut count = 0;
    while written <= SPOOL_BOUND {
        let text = line(&format!("s{count}"));
        written += text.len() as u64;
        append(spool, &text);
        count += 1;
    }
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
fn a_fully_consumed_spool_over_the_bound_is_truncated_and_later_records_apply() {
    let temp = private_temp();
    let instance = SpoolInstance::create(temp.path()).unwrap();
    let spool = instance.create_spool("tab-1").unwrap();
    let mut reader = SpoolReader::new(&spool);

    let count = fill_past_the_bound(&spool);
    assert!(fs::metadata(&spool).unwrap().len() > SPOOL_BOUND);
    let records = reader.poll();
    assert_eq!(
        records.len(),
        count,
        "every record is applied before truncation"
    );
    assert_eq!(
        fs::metadata(&spool).unwrap().len(),
        0,
        "a fully consumed spool over the bound is truncated"
    );

    append(&spool, &line("after-truncation"));
    let later = reader.poll();
    assert_eq!(later.len(), 1, "later records still apply");
    assert_eq!(later[0].session_id, "after-truncation");
}

#[test]
fn an_unconsumed_spool_over_the_bound_is_not_truncated() {
    let temp = private_temp();
    let instance = SpoolInstance::create(temp.path()).unwrap();
    let spool = instance.create_spool("tab-1").unwrap();
    let mut reader = SpoolReader::new(&spool);
    fill_past_the_bound(&spool);
    // A trailing partial record is not consumed, so the file is not fully consumed.
    append(&spool, "{\"sessionId\":\"par");
    let before = fs::metadata(&spool).unwrap().len();
    let _ = reader.poll();
    assert_eq!(fs::metadata(&spool).unwrap().len(), before);
}

#[cfg(unix)]
#[test]
fn truncation_waits_for_the_hook_lock() {
    let temp = private_temp();
    let instance = SpoolInstance::create(temp.path()).unwrap();
    let spool = instance.create_spool("tab-1").unwrap();
    let mut reader = SpoolReader::new(&spool);
    fill_past_the_bound(&spool);
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
