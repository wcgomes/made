//! SSH password reuse for one app process.
//!
//! The value lives only in this process's memory. It is never written to the
//! server registry, settings, diagnostics, logs, argv, or environment. A failed
//! authentication forgets it, and dropping the process is the only lifecycle
//! event that clears the remainder.
//!
//! Only password prompts participate. Key passphrases and host-key approvals
//! keep their explicit one-shot consent flow.

use std::sync::Mutex;

use made_platform::ssh_askpass::PromptAnswer;
use made_remote::protocol::AuthorityId;

/// The exact connection target a password was validated against: the saved
/// record and its `[user@]host[:port]` at that time. A tab that reconnects
/// with an older snapshot of an edited record never receives the new value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CacheKey {
    authority: AuthorityId,
    target: String,
}

impl CacheKey {
    pub(crate) fn new(
        authority: AuthorityId,
        metadata: &crate::ssh_servers::ServerMetadata,
    ) -> Self {
        Self {
            authority,
            target: crate::ssh_servers::host_address(metadata),
        }
    }
}

struct Entry {
    key: CacheKey,
    secret: Vec<u8>,
}

static PASSWORDS: Mutex<Vec<Entry>> = Mutex::new(Vec::new());

/// Whether this askpass text is the server password prompt. Host-key and key
/// passphrase prompts deliberately do not match.
pub fn is_password_prompt(text: &str) -> bool {
    text.to_ascii_lowercase().contains("password")
}

/// Replace the cached password for one connection target. Empty input never
/// stores.
pub(crate) fn remember(key: &CacheKey, secret: &str) {
    if secret.is_empty() {
        return;
    }
    let Ok(mut entries) = PASSWORDS.lock() else {
        return;
    };
    if let Some(entry) = entries.iter_mut().find(|entry| entry.key == *key) {
        entry.secret.fill(0);
        entry.secret.clear();
        entry.secret.extend_from_slice(secret.as_bytes());
        return;
    }
    entries.push(Entry {
        key: key.clone(),
        secret: secret.as_bytes().to_vec(),
    });
}

/// A copy of the cached password for exactly `key`.
fn cached_copy(key: &CacheKey) -> Option<Vec<u8>> {
    let entries = PASSWORDS.lock().ok()?;
    Some(
        entries
            .iter()
            .find(|entry| entry.key == *key)?
            .secret
            .clone(),
    )
}

/// Build a one-shot answer from the cached password without removing it.
#[cfg(test)]
pub(crate) fn answer(key: &CacheKey) -> Option<PromptAnswer> {
    PromptAnswer::new(cached_copy(key)?).ok()
}

/// Whether any password is cached for this saved server.
#[cfg(test)]
pub(crate) fn cached(authority: AuthorityId) -> bool {
    PASSWORDS
        .lock()
        .is_ok_and(|entries| entries.iter().any(|entry| entry.key.authority == authority))
}

/// Drop every cached password of a saved server after an edit or removal.
pub fn forget_server(authority: AuthorityId) {
    let Ok(mut entries) = PASSWORDS.lock() else {
        return;
    };
    entries.retain_mut(|entry| {
        let keep = entry.key.authority != authority;
        if !keep {
            entry.secret.fill(0);
        }
        keep
    });
}

/// Drop the cached password of `key` only while it is still `rejected`, the
/// value an attempt sent and saw refused. A newer value another attempt
/// validated in the meantime stays. Compared in constant time.
fn forget_if_still(key: &CacheKey, rejected: &[u8]) {
    let Ok(mut entries) = PASSWORDS.lock() else {
        return;
    };
    if let Some(index) = entries
        .iter()
        .position(|entry| entry.key == *key && constant_time_eq(&entry.secret, rejected))
    {
        let mut entry = entries.remove(index);
        entry.secret.fill(0);
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

/// Whether `prompt` is past the deadline it carries. OpenSSH no longer
/// waits for an answer to it, so nothing is sent and nothing is held.
pub(crate) fn expired(prompt: &made_platform::ssh_askpass::PromptRequest) -> bool {
    std::time::Instant::now() >= prompt.expires_at
}

/// Zero a draft's bytes before letting it go.
pub(crate) fn wipe(text: String) {
    let mut bytes = text.into_bytes();
    bytes.fill(0);
}

/// Zero a draft in place and leave it empty.
pub(crate) fn wipe_in_place(text: &mut String) {
    wipe(std::mem::take(text));
}

/// Whether `prompt` asks for the server password, the only prompt this cache
/// answers.
fn is_cached_prompt(prompt: &made_platform::ssh_askpass::PromptRequest) -> bool {
    matches!(
        crate::remote_session::classify_prompt(&prompt.text),
        crate::remote_session::SshPromptClass::Secret { .. }
    ) && is_password_prompt(&prompt.text)
}

/// The password handling of one SSH attempt to one connection target. A
/// password the user types joins the process cache only once that attempt
/// authenticates. A password prompt that comes back within the attempt
/// rejects the last answer, so a cached value is sent at most once per
/// attempt and, while it is still the cached one, a rejected value is
/// forgotten. Without a target it never caches or answers.
#[derive(Default)]
pub(crate) struct AttemptSecret {
    key: Option<CacheKey>,
    typed: Option<Vec<u8>>,
    /// The cached value this attempt sent.
    sent: Option<Vec<u8>>,
}

impl AttemptSecret {
    pub(crate) fn new(key: CacheKey) -> Self {
        Self {
            key: Some(key),
            typed: None,
            sent: None,
        }
    }

    /// A prompt arrived: the cached password to answer it with, or `None` to
    /// ask the user.
    pub(crate) fn on_prompt(
        &mut self,
        prompt: &made_platform::ssh_askpass::PromptRequest,
    ) -> Option<PromptAnswer> {
        if !is_cached_prompt(prompt) || expired(prompt) {
            return None;
        }
        let key = self.key.clone()?;
        self.drop_typed();
        if self.sent.is_some() {
            self.forget_sent();
            return None;
        }
        let mut secret = cached_copy(&key)?;
        // Check what `PromptAnswer` refuses first: a refused copy would be
        // dropped there without being zeroed.
        if secret.len() > made_platform::ssh_askpass::MAX_ANSWER_BYTES
            || secret.iter().any(|byte| matches!(byte, 0 | b'\n' | b'\r'))
        {
            secret.fill(0);
            return None;
        }
        let Ok(answer) = PromptAnswer::new(secret.clone()) else {
            secret.fill(0);
            return None;
        };
        self.sent = Some(secret);
        Some(answer)
    }

    /// The user answered a password prompt with `text`.
    pub(crate) fn typed(&mut self, prompt: &made_platform::ssh_askpass::PromptRequest, text: &str) {
        if is_cached_prompt(prompt) {
            self.drop_typed();
            self.typed = Some(text.as_bytes().to_vec());
        }
    }

    /// The attempt authenticated: its typed password joins the cache, and the
    /// next attempt starts afresh.
    pub(crate) fn authenticated(&mut self) {
        if let (Some(key), Some(text)) = (
            self.key.as_ref(),
            self.typed
                .as_deref()
                .and_then(|typed| std::str::from_utf8(typed).ok()),
        ) {
            remember(key, text);
        }
        self.drop_typed();
        self.drop_sent();
    }

    /// The attempt ended in `class`; rejected or cancelled authentication
    /// forgets the cached value it sent. The next attempt starts afresh.
    pub(crate) fn failed(&mut self, class: Option<crate::remote_session::SshFailureClass>) {
        use crate::remote_session::SshFailureClass;
        if matches!(
            class,
            Some(SshFailureClass::AuthFailed | SshFailureClass::Cancelled)
        ) {
            self.forget_sent();
        }
        self.drop_typed();
        self.drop_sent();
    }

    fn forget_sent(&mut self) {
        if let (Some(key), Some(sent)) = (self.key.as_ref(), self.sent.as_deref()) {
            forget_if_still(key, sent);
        }
    }

    fn drop_typed(&mut self) {
        if let Some(mut typed) = self.typed.take() {
            typed.fill(0);
        }
    }

    fn drop_sent(&mut self) {
        if let Some(mut sent) = self.sent.take() {
            sent.fill(0);
        }
    }
}

impl Drop for AttemptSecret {
    fn drop(&mut self) {
        self.drop_typed();
        self.drop_sent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(authority: u8, host: &str) -> CacheKey {
        CacheKey {
            authority: AuthorityId([authority; 16]),
            target: host.to_owned(),
        }
    }

    #[test]
    fn password_is_reused_only_for_the_same_target() {
        let first = key(1, "lab");
        remember(&first, "plantsuite");
        assert!(answer(&first).is_some());
        assert!(answer(&key(2, "lab")).is_none(), "another server");
        assert!(
            answer(&key(1, "lab2")).is_none(),
            "the same record at an edited address"
        );
        forget_server(AuthorityId([1; 16]));
        assert!(answer(&first).is_none());
    }

    #[test]
    fn a_replaced_password_is_exactly_the_new_one() {
        let target = key(4, "lab");
        remember(&target, "first-password");
        remember(&target, "second");
        assert_eq!(cached_copy(&target).as_deref(), Some(&b"second"[..]));
        forget_server(AuthorityId([4; 16]));
    }

    #[test]
    fn a_stale_rejection_keeps_a_newer_validated_value() {
        let target = key(5, "lab");
        remember(&target, "old");
        forget_if_still(&target, b"new");
        assert!(answer(&target).is_some(), "a different value stays");
        forget_if_still(&target, b"old");
        assert!(answer(&target).is_none());
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn empty_passwords_and_non_password_prompts_do_not_participate() {
        let target = key(3, "lab");
        remember(&target, "");
        assert!(answer(&target).is_none());
        assert!(is_password_prompt("vscode@host's password: "));
        assert!(!is_password_prompt(
            "Enter passphrase for key '/home/vscode/.ssh/id_ed25519':"
        ));
        assert!(!is_password_prompt(
            "The authenticity of host 'host' can't be established."
        ));
    }
}
