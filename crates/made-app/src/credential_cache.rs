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

struct Entry {
    authority: AuthorityId,
    secret: Vec<u8>,
}

static PASSWORDS: Mutex<Vec<Entry>> = Mutex::new(Vec::new());

/// Whether this askpass text is the server password prompt. Host-key and key
/// passphrase prompts deliberately do not match.
pub fn is_password_prompt(text: &str) -> bool {
    text.to_ascii_lowercase().contains("password")
}

/// Replace the cached password for one saved server. Empty input never stores.
pub fn remember(authority: AuthorityId, secret: &str) {
    if secret.is_empty() {
        return;
    }
    let Ok(mut entries) = PASSWORDS.lock() else {
        return;
    };
    if let Some(entry) = entries
        .iter_mut()
        .find(|entry| entry.authority == authority)
    {
        entry.secret.fill(0);
        entry.secret.extend_from_slice(secret.as_bytes());
        return;
    }
    entries.push(Entry {
        authority,
        secret: secret.as_bytes().to_vec(),
    });
}

/// Build a one-shot answer from the cached password without removing it.
pub fn answer(authority: AuthorityId) -> Option<PromptAnswer> {
    let entries = PASSWORDS.lock().ok()?;
    let entry = entries.iter().find(|entry| entry.authority == authority)?;
    PromptAnswer::new(entry.secret.clone()).ok()
}

/// Drop the cached password after an authentication failure or a server edit.
pub fn forget(authority: AuthorityId) {
    let Ok(mut entries) = PASSWORDS.lock() else {
        return;
    };
    if let Some(index) = entries
        .iter()
        .position(|entry| entry.authority == authority)
    {
        let mut entry = entries.remove(index);
        entry.secret.fill(0);
    }
}

#[cfg(test)]
mod tests {
    use super::{answer, forget, is_password_prompt, remember};
    use made_remote::protocol::AuthorityId;

    #[test]
    fn password_is_reused_only_for_the_same_authority() {
        let first = AuthorityId([1; 16]);
        let second = AuthorityId([2; 16]);
        remember(first, "plantsuite");
        assert!(answer(first).is_some());
        assert!(answer(second).is_none());
        forget(first);
        assert!(answer(first).is_none());
    }

    #[test]
    fn empty_passwords_and_non_password_prompts_do_not_participate() {
        let authority = AuthorityId([3; 16]);
        remember(authority, "");
        assert!(answer(authority).is_none());
        assert!(is_password_prompt("vscode@host's password: "));
        assert!(!is_password_prompt(
            "Enter passphrase for key '/home/vscode/.ssh/id_ed25519':"
        ));
        assert!(!is_password_prompt(
            "The authenticity of host 'host' can't be established."
        ));
    }
}
