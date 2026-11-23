//! UTF-8 character-type locale for PTY children.
//!
//! Under the `C` locale readline and zle edit bytes, not characters, and
//! macOS `/bin/bash` 3.2 drops 8-bit input outright, so accented text typed
//! into a terminal does not reach the shell. Remote supervisors and local
//! spawns both make sure the child decodes input as UTF-8; they differ only in
//! how far they trust a UTF-8 locale already in the environment.
use std::{
    ffi::OsString,
    io::Read,
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};

/// Value of the first non-empty variable that decides `LC_CTYPE`
/// (`LC_ALL`, `LC_CTYPE`, `LANG`), if any.
fn effective_ctype(env: &[(OsString, OsString)]) -> Option<&OsString> {
    ["LC_ALL", "LC_CTYPE", "LANG"].into_iter().find_map(|name| {
        env.iter()
            .rev()
            .find(|(key, value)| key == name && !value.is_empty())
            .map(|(_, value)| value)
    })
}

fn is_utf8_locale(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("utf-8") || name.contains("utf8")
}

fn lc_all_set(env: &[(OsString, OsString)]) -> bool {
    env.iter()
        .any(|(key, value)| key == "LC_ALL" && !value.is_empty())
}

/// `locale -a` spells the same locale `C.utf8` or `C.UTF-8` by libc.
fn locale_key(name: &str) -> String {
    name.to_ascii_lowercase().replace('-', "")
}

/// The UTF-8 entries of `installed`, in order.
fn utf8_locales(installed: Vec<String>) -> Vec<String> {
    installed
        .into_iter()
        .filter(|name| is_utf8_locale(name))
        .collect()
}

/// The preferred UTF-8 locale among `installed` (already UTF-8 only): the
/// entries of `preferred` in order, then the first listed.
fn pick_utf8_locale<'a>(installed: &'a [String], preferred: [&str; 2]) -> Option<&'a String> {
    let pick = |wanted: &str| {
        installed
            .iter()
            .find(|name| locale_key(name) == locale_key(wanted))
    };
    preferred
        .into_iter()
        .find_map(pick)
        .or_else(|| installed.first())
}

/// Make the remote child decode input as UTF-8 so accented characters typed in
/// the client reach the shell (readline and zle edit bytes, not characters,
/// under the `C` locale). A UTF-8 locale that the SSH session already carries,
/// for example a client `LANG` accepted through `AcceptEnv`, is kept only when
/// the server's own `locale -a` (`installed`) lists it: glibc has no locale
/// named plain `UTF-8` and silently falls back to `C`. Otherwise `LC_CTYPE`
/// is set, replacing any previous value including an explicit `LC_CTYPE=C`,
/// from `installed`, preferring `C.UTF-8` and then `en_US.UTF-8`. A locale the
/// server does not list is never invented, a non-empty `LC_ALL` is the user's
/// choice and is left alone, and message languages (`LANG`, `LC_MESSAGES`) are
/// not touched. If the server lists no locales, nothing changes.
pub fn ensure_utf8_locale(
    env: &mut Vec<(OsString, OsString)>,
    installed: impl FnOnce() -> Vec<String>,
) {
    if lc_all_set(env) {
        return;
    }
    let installed = utf8_locales(installed());
    if effective_ctype(env).is_some_and(|value| {
        let value = value.to_string_lossy();
        is_utf8_locale(&value)
            && installed
                .iter()
                .any(|name| locale_key(name) == locale_key(&value))
    }) {
        return;
    }
    let Some(choice) = pick_utf8_locale(&installed, ["C.UTF-8", "en_US.UTF-8"]) else {
        return;
    };
    env.retain(|(key, _)| key != "LC_CTYPE");
    env.push(("LC_CTYPE".into(), choice.into()));
}

/// The `LC_CTYPE` a local PTY child needs so it decodes input as UTF-8, or
/// `None` when `env` (the child's complete environment) needs nothing.
///
/// An app started from Finder or a desktop launcher often inherits no locale
/// at all. A UTF-8 locale already in effect (`LC_ALL`, `LC_CTYPE`, `LANG`,
/// first non-empty wins) is the user's and is trusted by name: on this host,
/// unlike a remote server, it is the same libc that will read it, and macOS
/// accepts a plain `UTF-8` that `locale -a` does not list. Otherwise the value
/// comes from `installed`, preferring `en_US.UTF-8`, then `C.UTF-8`, then the
/// first UTF-8 entry: the variable can travel on (macOS `ssh_config` sends
/// `LC_*`), and older servers have no `C.UTF-8`. As in [`ensure_utf8_locale`],
/// a non-empty `LC_ALL` is left alone even when it is not UTF-8 (it overrides
/// `LC_CTYPE`, so setting one would not help), and `installed` is not
/// consulted unless a value is needed.
pub fn local_utf8_ctype(
    env: &[(OsString, OsString)],
    installed: impl FnOnce() -> Vec<String>,
) -> Option<String> {
    if lc_all_set(env)
        || effective_ctype(env).is_some_and(|value| is_utf8_locale(&value.to_string_lossy()))
    {
        return None;
    }
    pick_utf8_locale(&utf8_locales(installed()), ["en_US.UTF-8", "C.UTF-8"]).cloned()
}

/// [`installed_locales`] of this host, queried once per process so that later
/// terminal spawns do not wait for `locale -a` again.
pub fn host_installed_locales() -> Vec<String> {
    static INSTALLED: OnceLock<Vec<String>> = OnceLock::new();
    INSTALLED.get_or_init(installed_locales).clone()
}

/// This host's installed locale names. Bounded in time and size; any failure
/// reads as "none installed", which leaves the environment unchanged.
pub fn installed_locales() -> Vec<String> {
    use std::process::{Command, Stdio};
    const LIMIT: u64 = 256 * 1024;
    let Ok(mut child) = Command::new("locale")
        .arg("-a")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    let Some(stdout) = child.stdout.take() else {
        return Vec::new();
    };
    let reader = thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.take(LIMIT).read_to_string(&mut text);
        text
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    reader
        .join()
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::local_utf8_ctype;
    use std::ffi::OsString;

    fn env(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect()
    }

    fn macos() -> Vec<String> {
        ["C", "POSIX", "en_US.UTF-8", "C.UTF-8", "pt_BR.ISO8859-1"]
            .map(String::from)
            .to_vec()
    }

    fn ctype(pairs: &[(&str, &str)]) -> Option<String> {
        local_utf8_ctype(&env(pairs), macos)
    }

    #[test]
    fn no_locale_gets_a_listed_utf8_ctype() {
        assert_eq!(ctype(&[]).as_deref(), Some("en_US.UTF-8"));
    }

    #[test]
    fn empty_values_count_as_unset() {
        assert_eq!(ctype(&[("LANG", "")]).as_deref(), Some("en_US.UTF-8"));
        assert_eq!(
            ctype(&[("LC_ALL", ""), ("LC_CTYPE", ""), ("LANG", "")]).as_deref(),
            Some("en_US.UTF-8")
        );
    }

    #[test]
    fn non_utf8_lang_or_ctype_gets_a_utf8_ctype() {
        for value in ["C", "POSIX", "pt_BR.ISO8859-1"] {
            assert_eq!(ctype(&[("LANG", value)]).as_deref(), Some("en_US.UTF-8"));
            assert_eq!(
                ctype(&[("LC_CTYPE", value)]).as_deref(),
                Some("en_US.UTF-8")
            );
        }
        // LC_CTYPE outranks a UTF-8 LANG.
        assert_eq!(
            ctype(&[("LANG", "en_US.UTF-8"), ("LC_CTYPE", "C")]).as_deref(),
            Some("en_US.UTF-8")
        );
    }

    #[test]
    fn existing_utf8_locale_is_kept_by_name() {
        assert_eq!(ctype(&[("LANG", "pt_BR.UTF-8")]), None);
        assert_eq!(ctype(&[("LC_CTYPE", "UTF-8")]), None);
        assert_eq!(ctype(&[("LANG", "C"), ("LC_CTYPE", "en_US.UTF-8")]), None);
        assert_eq!(ctype(&[("LANG", "de_DE.utf8")]), None);
        assert_eq!(ctype(&[("LANG", "C.Utf-8")]), None);
    }

    #[test]
    fn lc_all_is_never_overridden() {
        assert_eq!(ctype(&[("LC_ALL", "en_US.UTF-8"), ("LANG", "C")]), None);
        // A non-UTF-8 LC_ALL overrides LC_CTYPE, so nothing is set.
        assert_eq!(ctype(&[("LC_ALL", "C")]), None);
    }

    #[test]
    fn selection_follows_what_is_installed() {
        let pick = |installed: &[&str]| {
            let installed: Vec<String> = installed.iter().map(|s| (*s).into()).collect();
            local_utf8_ctype(&[], || installed)
        };
        assert_eq!(
            pick(&["C", "C.utf8", "en_US.utf8"]).as_deref(),
            Some("en_US.utf8")
        );
        assert_eq!(
            pick(&["C", "fr_FR.UTF-8", "C.utf8"]).as_deref(),
            Some("C.utf8")
        );
        assert_eq!(
            pick(&["C", "fr_FR.UTF-8", "en_US.UTF-8"]).as_deref(),
            Some("en_US.UTF-8")
        );
        assert_eq!(pick(&["C", "fr_FR.utf8"]).as_deref(), Some("fr_FR.utf8"));
        assert_eq!(pick(&["C", "POSIX"]), None);
        assert_eq!(pick(&[]), None);
    }

    #[test]
    fn installed_locales_are_not_queried_when_nothing_is_needed() {
        let env = env(&[("LANG", "en_US.UTF-8")]);
        assert_eq!(
            local_utf8_ctype(&env, || panic!("locale -a must not run")),
            None
        );
    }
}
