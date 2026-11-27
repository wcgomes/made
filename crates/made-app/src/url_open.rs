//! Per-pane confirmation for URLs a terminal child asks to open.
//!
//! A local child reaches the pane through `BROWSER` and the pane's private
//! request sink; a remote child arrives as the companion's `OpenUrlRequest`
//! push. Either way the pane shows one inline card with Open / Copy /
//! Dismiss. Nothing opens without the user's Open, and no URL is logged:
//! diagnostics carry counts only.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use made_platform::open_url::{SafeUrl, UrlRequestSink, UrlToken};
use made_platform::AppPaths;

use crate::integrations::{browser_contribution, EnvContribution};

/// About one card per second for a local pane.
pub const LOCAL_WINDOW: Duration = Duration::from_secs(1);
/// The companion already limits a remote tab to about one request per second;
/// the client window is looser so network jitter does not drop a request the
/// companion acknowledged, while still bounding a hostile server.
pub const REMOTE_WINDOW: Duration = Duration::from_millis(500);
/// Open stays disabled this long after a card appears or changes, so a click
/// aimed at one URL cannot land on a replacement.
pub const OPEN_ARM_DELAY: Duration = Duration::from_millis(400);

/// Where a request came from. A remote card names its server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlOrigin {
    Local,
    Remote(String),
}

/// The one visible card of a pane.
#[derive(Debug, Clone)]
pub struct UrlCard {
    pub url: SafeUrl,
    pub origin: UrlOrigin,
    pub shown_at: Instant,
}

impl UrlCard {
    /// Whether Open is enabled yet.
    pub fn armed(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.shown_at) >= OPEN_ARM_DELAY
    }
}

/// The user's choice on the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlCardAction {
    Open,
    Copy,
    Dismiss,
}

/// What the UI must do after an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlEffect {
    /// Hand the URL to the host browser.
    Open(SafeUrl),
    /// Put the URL text on the clipboard.
    Copy(String),
}

/// Outcome of offering a request to a pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    /// A new card is visible.
    Shown,
    /// The same URL is already on the card.
    Coalesced,
    /// Rate limited: the child prints the URL instead.
    Refused,
}

/// The card state of one pane: at most one card, about one new card per
/// window, duplicates of the visible card coalesced.
#[derive(Debug, Default)]
pub struct UrlPrompt {
    card: Option<UrlCard>,
    last_shown: Option<Instant>,
}

impl UrlPrompt {
    pub fn card(&self) -> Option<&UrlCard> {
        self.card.as_ref()
    }

    pub fn offer(&mut self, url: SafeUrl, origin: UrlOrigin, now: Instant) -> Offer {
        if self
            .card
            .as_ref()
            .is_some_and(|card| card.url == url && card.origin == origin)
        {
            return Offer::Coalesced;
        }
        let window = match origin {
            UrlOrigin::Local => LOCAL_WINDOW,
            UrlOrigin::Remote(_) => REMOTE_WINDOW,
        };
        if self
            .last_shown
            .is_some_and(|at| now.saturating_duration_since(at) < window)
        {
            return Offer::Refused;
        }
        self.last_shown = Some(now);
        self.card = Some(UrlCard {
            url,
            origin,
            shown_at: now,
        });
        Offer::Shown
    }

    /// Apply the user's action. Open before the card is armed does nothing.
    pub fn act(&mut self, action: UrlCardAction, now: Instant) -> Option<UrlEffect> {
        let card = self.card.as_ref()?;
        match action {
            UrlCardAction::Open if !card.armed(now) => None,
            UrlCardAction::Open => self.card.take().map(|card| UrlEffect::Open(card.url)),
            UrlCardAction::Copy => self
                .card
                .take()
                .map(|card| UrlEffect::Copy(card.url.as_str().to_owned())),
            UrlCardAction::Dismiss => {
                self.card = None;
                None
            }
        }
    }
}

/// Where the packaged helper would live: beside the application executable,
/// like the askpass helper. Absent means no `BROWSER` is set.
pub fn packaged_url_helper() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe
        .parent()?
        .join(made_platform::open_url::url_helper_file_name());
    path.is_file().then_some(path)
}

/// A local pane's endpoint, current launch token and card.
#[derive(Debug, Default)]
pub struct PaneUrlOpen {
    sink: Option<UrlRequestSink>,
    token: Option<UrlToken>,
    prompt: UrlPrompt,
    shown: u64,
    refused: u64,
}

impl PaneUrlOpen {
    pub fn prompt(&self) -> &UrlPrompt {
        &self.prompt
    }

    pub fn prompt_mut(&mut self) -> &mut UrlPrompt {
        &mut self.prompt
    }

    /// Cards shown and requests refused on this pane, for diagnostics.
    pub fn counts(&self) -> (u64, u64) {
        (self.shown, self.refused)
    }

    /// The core `BROWSER` contribution for a new local launch. Every launch
    /// gets a fresh token, so the children of an earlier launch are stale.
    /// Without the helper or an endpoint the variables are stripped instead.
    pub fn launch_contribution(
        &mut self,
        paths: Option<&AppPaths>,
        helper: Option<&Path>,
        inherited_browser: Option<&OsStr>,
    ) -> EnvContribution {
        self.token = UrlToken::generate().ok();
        if self.sink.is_none() {
            self.sink = paths.and_then(|paths| {
                made_platform::ensure_private_directory(&paths.temp_dir).ok()?;
                UrlRequestSink::new(&paths.temp_dir).ok()
            });
        }
        match (helper, self.sink.as_ref(), self.token.as_ref()) {
            (Some(helper), Some(sink), Some(token)) => {
                browser_contribution(Some((helper, sink.socket_path(), token)), inherited_browser)
            }
            _ => browser_contribution(None, inherited_browser),
        }
    }

    /// Route one request by token: only the current launch's token reaches
    /// the card. Returns whether the helper is acknowledged.
    pub fn admit(&mut self, token: &str, url: SafeUrl, origin: UrlOrigin, now: Instant) -> bool {
        if !self.token.as_ref().is_some_and(|own| own.matches(token)) {
            self.refused += 1;
            return false;
        }
        self.offer(url, origin, now)
    }

    /// Offer a request that already passed its transport's routing (a remote
    /// push for this tab). Returns whether it is on the card.
    pub fn offer(&mut self, url: SafeUrl, origin: UrlOrigin, now: Instant) -> bool {
        match self.prompt.offer(url, origin, now) {
            Offer::Shown => {
                self.shown += 1;
                true
            }
            Offer::Coalesced => true,
            Offer::Refused => {
                self.refused += 1;
                false
            }
        }
    }

    /// Drain this pane's sink. Returns how many new cards appeared.
    pub fn poll(&mut self, now: Instant) -> usize {
        let Some(sink) = self.sink.as_mut() else {
            return 0;
        };
        let requests = sink.poll();
        let before = self.shown;
        for request in requests {
            let url = request.url().clone();
            let token = request.token().to_owned();
            if self.admit(&token, url, UrlOrigin::Local, now) {
                request.accept();
            } else {
                request.reject();
            }
        }
        (self.shown - before) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use made_platform::open_url::validate_url;

    fn url(text: &str) -> SafeUrl {
        validate_url(text).unwrap()
    }

    #[test]
    fn nothing_opens_without_an_armed_open_and_each_action_maps_once() {
        let start = Instant::now();
        let mut prompt = UrlPrompt::default();
        assert_eq!(prompt.act(UrlCardAction::Open, start), None);
        assert_eq!(
            prompt.offer(url("https://a.example/"), UrlOrigin::Local, start),
            Offer::Shown
        );
        // Showing a card is not opening it, and an early Open is ignored.
        assert_eq!(prompt.act(UrlCardAction::Open, start), None);
        assert!(prompt.card().is_some());
        let armed = start + OPEN_ARM_DELAY;
        assert_eq!(
            prompt.act(UrlCardAction::Open, armed),
            Some(UrlEffect::Open(url("https://a.example/")))
        );
        assert!(prompt.card().is_none());
        assert_eq!(prompt.act(UrlCardAction::Open, armed), None);

        let later = start + Duration::from_secs(2);
        prompt.offer(url("https://b.example/x"), UrlOrigin::Local, later);
        assert_eq!(
            prompt.act(UrlCardAction::Copy, later),
            Some(UrlEffect::Copy("https://b.example/x".into()))
        );
        assert!(prompt.card().is_none());

        let later = later + Duration::from_secs(2);
        prompt.offer(url("https://c.example/"), UrlOrigin::Local, later);
        assert_eq!(
            prompt.act(UrlCardAction::Dismiss, later + OPEN_ARM_DELAY),
            None
        );
        assert!(prompt.card().is_none());
    }

    #[test]
    fn one_card_coalesced_and_rate_limited() {
        let start = Instant::now();
        let mut prompt = UrlPrompt::default();
        let a = url("https://a.example/");
        assert_eq!(
            prompt.offer(a.clone(), UrlOrigin::Local, start),
            Offer::Shown
        );
        let soon = start + Duration::from_millis(100);
        assert_eq!(
            prompt.offer(a.clone(), UrlOrigin::Local, soon),
            Offer::Coalesced
        );
        assert_eq!(
            prompt.offer(url("https://b.example/"), UrlOrigin::Local, soon),
            Offer::Refused
        );
        assert_eq!(prompt.card().unwrap().url, a);
        // After the window a new URL replaces the card and re-arms Open.
        let later = start + LOCAL_WINDOW;
        assert_eq!(
            prompt.offer(url("https://b.example/"), UrlOrigin::Local, later),
            Offer::Shown
        );
        assert!(!prompt.card().unwrap().armed(later));
        // A remote card names its server and uses the remote window.
        let remote = UrlOrigin::Remote("build-box".into());
        let r = later + REMOTE_WINDOW;
        assert_eq!(prompt.offer(a.clone(), remote.clone(), r), Offer::Shown);
        assert_eq!(prompt.card().unwrap().origin, remote);
    }

    #[test]
    fn requests_route_only_with_the_current_launch_token() {
        let start = Instant::now();
        let mut pane = PaneUrlOpen::default();
        // No launch yet: nothing is admitted.
        assert!(!pane.admit(
            &"0".repeat(32),
            url("https://a.example/"),
            UrlOrigin::Local,
            start
        ));
        let helper = Path::new("/opt/made/made-open");
        let _ = pane.launch_contribution(None, Some(helper), None);
        let first = pane.token.clone().unwrap();
        let foreign = UrlToken::generate().unwrap();
        assert!(!pane.admit(
            foreign.as_str(),
            url("https://a.example/"),
            UrlOrigin::Local,
            start
        ));
        assert!(pane.prompt().card().is_none());
        // A relaunch rotates the token: the earlier launch's children are stale.
        let _ = pane.launch_contribution(None, Some(helper), None);
        assert!(!pane.admit(
            first.as_str(),
            url("https://a.example/"),
            UrlOrigin::Local,
            start
        ));
        let current = pane.token.clone().unwrap();
        assert!(pane.admit(
            current.as_str(),
            url("https://a.example/"),
            UrlOrigin::Local,
            start
        ));
        assert_eq!(pane.counts(), (1, 3));
    }

    #[test]
    fn browser_is_set_only_with_a_helper_and_an_endpoint() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let paths = AppPaths {
            install_root: root.join("install"),
            resource_dir: root.join("install"),
            data_dir: root.join("data"),
            plugin_root: root.join("plugin"),
            plugin_dir: root.join("plugin").join("opencode-made"),
            shipped_plugins_dir: None,
            plugin_source: made_platform::PluginDirSource::DevelopmentRepository,
            opencode_config_path: root.join("config").join("cli.json"),
            log_dir: root.join("log"),
            temp_dir: root.join("temp"),
        };
        let helper = temp.path().join("made-open");
        let get = |c: &EnvContribution, key: &str| {
            c.set
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.to_string_lossy().into_owned())
        };
        let mut pane = PaneUrlOpen::default();
        let set = pane.launch_contribution(Some(&paths), Some(&helper), Some(OsStr::new("lynx")));
        assert_eq!(
            get(&set, "BROWSER").as_deref(),
            Some(helper.to_str().unwrap())
        );
        assert_eq!(get(&set, "MADE_ORIGINAL_BROWSER").as_deref(), Some("lynx"));
        assert!(get(&set, "MADE_URL_SOCKET").is_some());
        assert!(get(&set, "MADE_URL_TOKEN").is_some());

        let mut missing = PaneUrlOpen::default();
        let unset = missing.launch_contribution(Some(&paths), None, Some(OsStr::new("lynx")));
        assert!(unset.set.is_empty(), "the user's BROWSER stays untouched");
        assert!(unset.strip.contains(&"MADE_URL_SOCKET"));
        assert!(unset.strip.contains(&"MADE_URL_TOKEN"));
    }
}
