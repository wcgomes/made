//! Pins and validation of the integration list (AIA-03, AIA-05, AIA-09).
//!
//! The pin test asserts every descriptor field of the product list verbatim, so
//! a change to the list or to any identifier trips it. Validation is exercised
//! with lists defined only in this file.

use made_app::integrations::{
    validate, Descriptor, FollowKind, Integration, IntegrationList, ListError, Mark, Rank,
    CLAUDE_CODE, GROK, OPENCODE, PRODUCT,
};

#[test]
fn product_list_holds_exactly_the_three_integrations_in_catalog_order() {
    assert_eq!(PRODUCT.len(), 3);
    assert_eq!(PRODUCT[0], OPENCODE);
    assert_eq!(PRODUCT[1], GROK);
    assert_eq!(PRODUCT[2], CLAUDE_CODE);
    let keys: Vec<_> = PRODUCT.iter().map(|row| row.descriptor().key).collect();
    assert_eq!(keys, ["opencode", "grok", "claude-code"]);
    assert_eq!(validate(PRODUCT), Ok(()));
}

#[test]
fn every_descriptor_field_of_the_product_list_is_pinned() {
    let opencode = OPENCODE.descriptor();
    assert_eq!(opencode.key, "opencode");
    assert_eq!(opencode.program, "opencode");
    assert_eq!(opencode.label, "OpenCode");
    assert_eq!(opencode.menu_item_id, "opencode");
    assert_eq!(opencode.scope_id, "opencode-v2");
    assert_eq!(opencode.scope_name, "OpenCode v2");
    assert_eq!(opencode.root_source_id, Some("opencode"));
    assert_eq!(opencode.mark, Mark::Vendored("opencode.svg"));
    assert_eq!(opencode.rank, Rank(100));
    assert!(OPENCODE.launch_args().is_empty());

    let grok = GROK.descriptor();
    assert_eq!(grok.key, "grok");
    assert_eq!(grok.program, "grok");
    assert_eq!(grok.label, "Grok Build");
    assert_eq!(grok.menu_item_id, "grok");
    assert_eq!(grok.scope_id, "grok-build");
    assert_eq!(grok.scope_name, "Grok Build");
    assert_eq!(grok.root_source_id, Some("grok"));
    assert!(matches!(
        grok.mark,
        Mark::Image { uri: "bytes://grok.svg", bytes } if !bytes.is_empty()
    ));
    assert_eq!(grok.rank, Rank(300));
    assert_eq!(GROK.launch_args(), ["dashboard"]);

    let claude = CLAUDE_CODE.descriptor();
    assert_eq!(claude.key, "claude-code");
    assert_eq!(claude.program, "claude");
    assert_eq!(claude.label, "Claude Code");
    assert_eq!(claude.menu_item_id, "claude");
    assert_eq!(claude.scope_id, "claude-code");
    assert_eq!(claude.scope_name, "Claude Code");
    assert_eq!(claude.root_source_id, Some("claude-code"));
    assert!(matches!(
        claude.mark,
        Mark::Image { uri: "bytes://claude-code.svg", bytes } if !bytes.is_empty()
    ));
    assert_eq!(claude.rank, Rank(200));
    assert!(CLAUDE_CODE.launch_args().is_empty());
}

#[test]
fn rank_is_independent_of_catalog_order() {
    // Catalog order is OpenCode, Grok Build, Claude Code; precedence is
    // OpenCode, Claude Code, Grok Build.
    assert!(OPENCODE.descriptor().rank < CLAUDE_CODE.descriptor().rank);
    assert!(CLAUDE_CODE.descriptor().rank < GROK.descriptor().rank);
    let position = |tui| PRODUCT.iter().position(|row| *row == tui).unwrap();
    assert!(position(GROK) < position(CLAUDE_CODE));
}

#[test]
fn integration_references_are_equal_by_key() {
    assert_eq!(OPENCODE, OPENCODE);
    assert_ne!(OPENCODE, GROK);
    assert_ne!(GROK, CLAUDE_CODE);
}

/// A test-only integration that exists to exercise list validation.
struct Probe {
    descriptor: &'static Descriptor,
    follow: FollowKind,
}

impl Integration for Probe {
    fn descriptor(&self) -> &'static Descriptor {
        self.descriptor
    }

    fn follow(&self) -> FollowKind {
        self.follow
    }
}

const fn descriptor(key: &'static str, rank: u16) -> Descriptor {
    Descriptor {
        key,
        program: key,
        label: key,
        menu_item_id: key,
        scope_id: key,
        scope_name: key,
        root_source_id: None,
        mark: Mark::Vendored("probe.svg"),
        rank: Rank(rank),
    }
}

static FIRST: Descriptor = descriptor("first", 100);
static SAME_KEY: Descriptor = descriptor("first", 200);
static SAME_RANK: Descriptor = descriptor("second", 100);
static SECOND: Descriptor = descriptor("second", 200);
static THIRD: Descriptor = descriptor("third", 300);

static PROBE_FIRST: Probe = Probe {
    descriptor: &FIRST,
    follow: FollowKind::None,
};
static PROBE_SAME_KEY: Probe = Probe {
    descriptor: &SAME_KEY,
    follow: FollowKind::None,
};
static PROBE_SAME_RANK: Probe = Probe {
    descriptor: &SAME_RANK,
    follow: FollowKind::None,
};
static PROBE_SECOND: Probe = Probe {
    descriptor: &SECOND,
    follow: FollowKind::None,
};
static PROBE_THIRD_OWNER: Probe = Probe {
    descriptor: &THIRD,
    follow: FollowKind::SessionChannel,
};
static PROBE_FIRST_OWNER: Probe = Probe {
    descriptor: &FIRST,
    follow: FollowKind::SessionChannel,
};

#[test]
fn a_list_with_unique_keys_and_ranks_is_valid() {
    const LIST: IntegrationList = &[&PROBE_FIRST, &PROBE_SECOND];
    assert_eq!(validate(LIST), Ok(()));
}

#[test]
fn duplicate_keys_are_rejected() {
    const LIST: IntegrationList = &[&PROBE_FIRST, &PROBE_SAME_KEY];
    assert_eq!(validate(LIST), Err(ListError::DuplicateKey("first")));
}

#[test]
fn duplicate_ranks_are_rejected() {
    const LIST: IntegrationList = &[&PROBE_FIRST, &PROBE_SAME_RANK];
    assert_eq!(validate(LIST), Err(ListError::DuplicateRank(Rank(100))));
}

#[test]
fn a_session_channel_owner_must_be_the_lowest_ranked() {
    const REJECTED: IntegrationList = &[&PROBE_FIRST, &PROBE_THIRD_OWNER];
    assert_eq!(
        validate(REJECTED),
        Err(ListError::SessionChannelOwnerNotLowest("third"))
    );
    const ACCEPTED: IntegrationList = &[&PROBE_FIRST_OWNER, &PROBE_SECOND];
    assert_eq!(validate(ACCEPTED), Ok(()));
}

// ---- Neutrality and structure scans (AIA-03, AIA-04, AIA-09) ----

mod neutrality {
    use std::fs;
    use std::path::{Path, PathBuf};

    use made_app::integrations::PRODUCT;

    /// Identifiers of the legacy local Claude Code seams. Core files must not
    /// name them outside remote-path regions.
    const LEGACY_CLAUDE_CODE: &[&str] = &[
        "claude_root",
        "ClaudeRuntime",
        "loads_claude_plugin",
        "poll_claude",
        "prepare_claude_launch",
        "self.claude",
        "claude_follow::",
        "ClaudeTab",
    ];

    /// Identifiers of the legacy Grok Build seams. Core files must not name
    /// them outside remote-path regions.
    const LEGACY_GROK: &[&str] = &[
        "grok_root",
        "GrokLiveSession",
        "poll_grok_",
        "GrokDiagnosticsPhase",
        "grok_row_for",
    ];

    /// Identifiers of the legacy OpenCode seams. Core files must not name
    /// them outside remote-path regions.
    const LEGACY_OPENCODE: &[&str] = &[
        "workspace_opencode_active",
        "opencode_icon",
        "remember_opencode_missing",
        "note_opencode_",
    ];

    /// Whole files that are remote paths.
    const REMOTE_FILES: &[&str] = &["ssh_servers.rs", "remote_workspace.rs", "remote_session.rs"];

    /// Remote-path regions per core file (AIA-02). A new or removed region
    /// changes the count and trips the pin, so a region cannot be added to
    /// hide code from the scans without this list changing in review.
    const REGION_COUNTS: &[(&str, usize)] = &[
        ("app.rs", 10),
        ("diagnostics.rs", 2),
        ("tui.rs", 7),
        ("ui.rs", 10),
        ("workbench.rs", 14),
    ];

    /// Lines inside remote-path regions per core file (the lines between the
    /// markers, nested regions included). Widening a region changes the
    /// count and shows in the diff of this list.
    const EXEMPT_LINE_COUNTS: &[(&str, usize)] = &[
        ("app.rs", 1075),
        ("diagnostics.rs", 196),
        ("tui.rs", 40),
        ("ui.rs", 288),
        ("workbench.rs", 245),
    ];

    /// Core-named items that stay outside remote-path regions, in two named
    /// categories. A row is (file, integrations it excuses, text a line
    /// contains, lines that match). A line is excused only when every
    /// integration it names is among its row's integrations, so a row for
    /// OpenCode never excuses a Grok or Claude Code hit. An empty text excuses
    /// the whole file. The expected counts are pinned, so a new core-named
    /// line, or a stale row, fails the scan.
    type AllowedRow = (&'static str, &'static [&'static str], &'static str, usize);

    /// Category 1, the session-channel status vocabulary: it names OpenCode
    /// because OpenCode is the session-channel owner.
    const SESSION_CHANNEL_NAMES: &[AllowedRow] = &[
        // The session-channel status vocabulary: reason codes, their
        // sentences and the peer-version fields.
        ("integration_status.rs", &["opencode"], "", 36),
        // The peer version of the session-channel owner in the diagnostics
        // snapshot and its copied report.
        ("diagnostics.rs", &["opencode"], "opencode_version", 4),
        ("diagnostics.rs", &["opencode"], "opencode version", 1),
        // The session-channel owner's connected pill label, its version row
        // and the copy that names it in the sessions surfaces.
        ("ui.rs", &["opencode"], "opencode_pill_label", 2),
        ("ui.rs", &["opencode"], "\"OpenCode\",", 1),
        ("ui.rs", &["opencode"], ".opencode_version", 1),
        ("ui.rs", &["opencode"], "Open OpenCode in the terminal.", 2),
        ("ui.rs", &["opencode"], "All OpenCode sessions", 1),
        // The session-channel log lines.
        (
            "workbench.rs",
            &["opencode"],
            "OpenCode integration connected",
            1,
        ),
        ("workbench.rs", &["opencode"], "info.opencode_version", 1),
        (
            "workbench.rs",
            &["opencode"],
            "OpenCode integration rejected",
            1,
        ),
        (
            "workbench.rs",
            &["opencode"],
            "OpenCode integration unloaded",
            1,
        ),
    ];

    /// Category 2, preserved product copy / scope declaration (AIA-01): text
    /// and declarations that existed before this refactor and are kept
    /// verbatim. They name integrations as product copy or as a scope fact,
    /// not as session-channel vocabulary.
    const PRESERVED_PRODUCT_COPY: &[AllowedRow] = &[
        // The restart confirmation, which lists the sessions a restart closes.
        (
            "ui.rs",
            &["opencode", "grok"],
            "including OpenCode, Grok and SSH sessions",
            1,
        ),
        // The scope declaration that Made starts no OpenCode server of its own.
        (
            "integration_status.rs",
            &["opencode"],
            "separate_opencode_server",
            2,
        ),
    ];

    fn src_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
    }

    /// The integration modules, read from the `src/integrations/` directory
    /// listing: every folder except `shared`.
    fn integration_modules() -> Vec<String> {
        let mut modules: Vec<String> = fs::read_dir(src_dir().join("integrations"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_dir())
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .filter(|name| name != "shared")
            .collect();
        modules.sort();
        assert_eq!(
            modules.len(),
            PRODUCT.len(),
            "the integration folders and the product list differ: {modules:?}"
        );
        modules
    }

    /// Whether `name` is a path segment of `line`: a whole word followed by
    /// `::`.
    fn has_path_segment(line: &str, name: &str) -> bool {
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let mut from = 0;
        while let Some(found) = line[from..].find(name) {
            let start = from + found;
            let end = start + name.len();
            let before = line[..start].chars().next_back();
            if !before.is_some_and(is_word) && line[end..].trim_start().starts_with("::") {
                return true;
            }
            from = end;
        }
        false
    }

    /// Whether `text` holds `name` as a whole word.
    fn has_word(text: &str, name: &str) -> bool {
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let mut from = 0;
        while let Some(found) = text[from..].find(name) {
            let start = from + found;
            let end = start + name.len();
            let before = text[..start].chars().next_back();
            let after = text[end..].chars().next();
            if !before.is_some_and(is_word) && !after.is_some_and(is_word) {
                return true;
            }
            from = end;
        }
        false
    }

    /// Every way the given lines reach an integration module: a path through
    /// it, or a grouped, aliased or glob import of the integrations tree,
    /// including multi-line statements. Each entry is `number: reason`.
    fn import_violations(lines: &[(usize, &str)], modules: &[String]) -> Vec<String> {
        let mut found = Vec::new();
        for (number, line) in lines {
            for module in modules {
                if has_path_segment(line, module) {
                    found.push(format!("{number}: path `{module}::`"));
                }
            }
        }
        for (number, statement) in use_statements(lines) {
            if !statement.contains("integrations") {
                continue;
            }
            for module in modules {
                if has_word(&statement, module) {
                    found.push(format!("{number}: imports `{module}`"));
                }
            }
            if statement.contains("integrations as ") || statement.contains("integrations::*") {
                found.push(format!("{number}: aliases or globs the integrations tree"));
            }
        }
        found
    }

    /// The `use` statements among scanned lines, each joined into one string
    /// with its first line number.
    fn use_statements(lines: &[(usize, &str)]) -> Vec<(usize, String)> {
        let mut statements = Vec::new();
        let mut open: Option<(usize, String)> = None;
        for (number, line) in lines {
            let trimmed = line.trim_start();
            if is_comment(line) {
                continue;
            }
            if open.is_none() && strip_visibility(trimmed).starts_with("use ") {
                open = Some((*number, String::new()));
            }
            if let Some((_, text)) = open.as_mut() {
                text.push_str(trimmed);
                text.push(' ');
                if trimmed.ends_with(';') {
                    statements.push(open.take().unwrap());
                }
            }
        }
        statements
    }

    /// `text` without a leading visibility: `pub`, `pub(crate)`,
    /// `pub(super)`, `pub(self)` or `pub(in path)`.
    fn strip_visibility(text: &str) -> &str {
        let Some(rest) = text.strip_prefix("pub") else {
            return text;
        };
        let rest = match rest.strip_prefix('(') {
            Some(inner) => match inner.find(')') {
                Some(close) => &inner[close + 1..],
                None => return text,
            },
            None => rest,
        };
        if rest.starts_with(char::is_whitespace) {
            rest.trim_start()
        } else {
            text
        }
    }

    /// The core sources: every file of `src` outside `integrations/`, except
    /// the whole-file remote paths.
    fn core_files() -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = fs::read_dir(src_dir())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
            .filter(|path| {
                !REMOTE_FILES
                    .iter()
                    .any(|name| path.file_name().is_some_and(|file| file == *name))
            })
            .collect();
        files.sort();
        assert!(!files.is_empty(), "the scan read no core file");
        files
    }

    /// The lines of `source` outside remote-path regions, with their numbers.
    /// Panics when the markers do not pair up: an END without a BEGIN, or a
    /// BEGIN still open at the end of the source.
    fn scanned_lines(source: &str) -> Vec<(usize, &str)> {
        let mut depth = 0usize;
        let mut lines = Vec::new();
        for (index, line) in source.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("// BEGIN remote-path") {
                depth += 1;
                continue;
            }
            if trimmed.starts_with("// END remote-path") {
                assert!(
                    depth > 0,
                    "line {}: END remote-path without BEGIN",
                    index + 1
                );
                depth -= 1;
                continue;
            }
            if depth == 0 {
                lines.push((index + 1, line));
            }
        }
        assert_eq!(depth, 0, "a BEGIN remote-path has no END");
        lines
    }

    /// The part of a core file the scans read: everything before its unit
    /// tests. The unit-test module must be the last item of the file, so no
    /// production code can follow it unscanned.
    fn production_part(source: &str) -> String {
        let lines: Vec<&str> = source.lines().collect();
        let starts: Vec<usize> = lines
            .windows(2)
            .enumerate()
            .filter(|(_, pair)| {
                pair[0].trim_end() == "#[cfg(test)]" && pair[1].starts_with("mod tests")
            })
            .map(|(index, _)| index)
            .collect();
        assert!(starts.len() <= 1, "more than one `mod tests` block");
        let Some(&end) = starts.first() else {
            return lines.join("\n");
        };
        let close = lines[end..]
            .iter()
            .position(|line| *line == "}")
            .expect("the `mod tests` block never closes at column 0")
            + end;
        assert!(
            lines[close + 1..].iter().all(|line| line.trim().is_empty()),
            "the `mod tests` block is not the last item of the file"
        );
        lines[..end].join("\n")
    }

    /// The number of lines inside remote-path regions of `source`, marker
    /// lines excluded.
    fn exempt_lines(source: &str) -> usize {
        let total = source.lines().count();
        let markers = source
            .lines()
            .filter(|line| {
                let trimmed = line.trim_start();
                trimmed.starts_with("// BEGIN remote-path")
                    || trimmed.starts_with("// END remote-path")
            })
            .count();
        total - markers - scanned_lines(source).len()
    }

    fn is_comment(line: &str) -> bool {
        line.trim_start().starts_with("//")
    }

    #[test]
    fn core_files_do_not_name_the_legacy_claude_code_seams() {
        let mut found = Vec::new();
        for path in core_files() {
            let source = fs::read_to_string(&path).unwrap();
            for (number, line) in scanned_lines(&production_part(&source)) {
                if is_comment(line) {
                    continue;
                }
                for name in LEGACY_CLAUDE_CODE {
                    if line.contains(name) {
                        found.push(format!("{}:{number}: {name}", path.display()));
                    }
                }
            }
        }
        assert!(
            found.is_empty(),
            "legacy seams in core files:\n{}",
            found.join("\n")
        );
    }

    #[test]
    fn core_files_do_not_name_the_legacy_grok_seams() {
        let mut found = Vec::new();
        for path in core_files() {
            let source = fs::read_to_string(&path).unwrap();
            for (number, line) in scanned_lines(&production_part(&source)) {
                if is_comment(line) {
                    continue;
                }
                for name in LEGACY_GROK {
                    if line.contains(name) {
                        found.push(format!("{}:{number}: {name}", path.display()));
                    }
                }
            }
        }
        assert!(
            found.is_empty(),
            "legacy Grok seams in core files:\n{}",
            found.join("\n")
        );
    }

    #[test]
    fn core_files_do_not_name_the_legacy_opencode_seams() {
        let mut found = Vec::new();
        for path in core_files() {
            let source = fs::read_to_string(&path).unwrap();
            for (number, line) in scanned_lines(&production_part(&source)) {
                if is_comment(line) {
                    continue;
                }
                for name in LEGACY_OPENCODE {
                    if line.contains(name) {
                        found.push(format!("{}:{number}: {name}", path.display()));
                    }
                }
            }
        }
        assert!(
            found.is_empty(),
            "legacy OpenCode seams in core files:\n{}",
            found.join("\n")
        );
    }

    #[test]
    fn core_files_do_not_import_an_integration_module() {
        let modules = integration_modules();
        let mut found = Vec::new();
        for path in core_files() {
            let source = fs::read_to_string(&path).unwrap();
            let production = production_part(&source);
            let lines: Vec<(usize, &str)> = scanned_lines(&production)
                .into_iter()
                .filter(|(_, line)| !is_comment(line))
                .collect();
            for violation in import_violations(&lines, &modules) {
                found.push(format!("{}:{violation}", path.display()));
            }
        }
        assert!(
            found.is_empty(),
            "core imports of an integration module:\n{}",
            found.join("\n")
        );
    }

    #[test]
    fn the_import_scan_sees_grouped_aliased_and_deep_paths() {
        let modules = ["claude_code".to_owned(), "grok".to_owned()];
        let flagged = |source: &str| {
            let lines: Vec<(usize, &str)> = source.lines().enumerate().collect();
            !import_violations(&lines, &modules).is_empty()
        };
        assert!(flagged("use crate::integrations::{grok, AppCtx};"));
        assert!(flagged(
            "use crate::integrations::{\n    AppCtx,\n    claude_code,\n};"
        ));
        assert!(flagged("use crate::integrations as i;"));
        assert!(flagged("pub(super) use crate::integrations::grok as g;"));
        assert!(flagged(
            "pub(in crate::ui) use crate::integrations::{\n    grok,\n};"
        ));
        assert!(flagged("pub use crate::integrations::claude_code;"));
        assert!(!flagged(
            "pub(super) use crate::integrations::PRODUCT as p;"
        ));
        assert!(flagged("let x = i::grok::mark();"));
        assert!(flagged("crate::integrations::claude_code :: tab(w)"));
        assert!(!flagged("use crate::integrations::{AppCtx, PRODUCT};"));
        assert!(!flagged("let grokked = pregrok::x();"));
        assert!(!flagged("let claude_code_label = 1;"));
    }

    #[test]
    fn remote_path_regions_are_balanced_and_pinned_per_core_file() {
        for path in core_files() {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let source = fs::read_to_string(&path).unwrap();
            // Panics on an unpaired marker, or when the unit tests are not
            // the file's last item.
            let production = production_part(&source);
            scanned_lines(&production);
            // Regions in the unit tests would be dead markers.
            scanned_lines(&source);
            let regions = source
                .lines()
                .filter(|line| line.trim_start().starts_with("// BEGIN remote-path"))
                .count();
            let pinned = REGION_COUNTS
                .iter()
                .find(|(file, _)| *file == name)
                .map_or(0, |(_, count)| *count);
            assert_eq!(regions, pinned, "{name}: remote-path regions");
            let exempt = EXEMPT_LINE_COUNTS
                .iter()
                .find(|(file, _)| *file == name)
                .map_or(0, |(_, count)| *count);
            assert_eq!(
                exempt_lines(&production),
                exempt,
                "{name}: lines inside remote-path regions"
            );
        }
    }

    #[test]
    fn core_production_lines_name_no_integration_outside_pinned_exceptions() {
        // The integration names, from the module folders: the folder name and
        // its first word (`claude_code` is named `claude`).
        let mut names: Vec<String> = Vec::new();
        for module in integration_modules() {
            names.push(module.clone());
            if let Some(first) = module.split('_').next() {
                names.push(first.to_owned());
            }
        }
        names.sort();
        names.dedup();
        // The module an integration name belongs to: its folder name, or the
        // folder name's first word.
        let modules = integration_modules();
        let named_modules = |lowered: &str| -> Vec<&str> {
            modules
                .iter()
                .filter(|module| {
                    lowered.contains(module.as_str())
                        || module
                            .split('_')
                            .next()
                            .is_some_and(|first| lowered.contains(first))
                })
                .map(String::as_str)
                .collect()
        };
        // Preserved product copy is tried first, so a line it excuses is not
        // counted against the session-channel rows.
        let rows: Vec<&AllowedRow> = PRESERVED_PRODUCT_COPY
            .iter()
            .chain(SESSION_CHANNEL_NAMES)
            .collect();
        let mut hits = vec![0usize; rows.len()];
        let mut found = Vec::new();
        for path in core_files() {
            let file = path.file_name().unwrap().to_string_lossy().into_owned();
            let source = fs::read_to_string(&path).unwrap();
            for (number, line) in scanned_lines(&production_part(&source)) {
                if is_comment(line) {
                    continue;
                }
                let lowered = line.to_ascii_lowercase();
                if !names.iter().any(|name| lowered.contains(name.as_str())) {
                    continue;
                }
                let named = named_modules(&lowered);
                let row = rows.iter().position(|(allowed_file, excused, text, _)| {
                    *allowed_file == file
                        && line.contains(text)
                        && named.iter().all(|module| excused.contains(module))
                });
                match row {
                    Some(row) => hits[row] += 1,
                    None => found.push(format!("{file}:{number}: {}", line.trim())),
                }
            }
        }
        assert!(
            found.is_empty(),
            "core lines naming an integration outside remote-path regions, or \
             naming one their row does not excuse:\n{}",
            found.join("\n")
        );
        let drifted: Vec<String> = rows
            .iter()
            .enumerate()
            .filter(|(row, (_, _, _, expected))| hits[*row] != *expected)
            .map(|(row, (file, _, text, expected))| {
                format!(
                    "`{text}` in {file}: {} matching lines, pinned {expected}",
                    hits[row]
                )
            })
            .collect();
        assert!(
            drifted.is_empty(),
            "allowed core names drifted:\n{}",
            drifted.join("\n")
        );
    }

    #[test]
    fn no_transitional_shim_marker_remains() {
        // Written in two parts so this file does not carry the marker itself.
        const MARKER: &str = concat!("transitional", "-shim (AIA-11)");
        let mut files = Vec::new();
        module_files(&src_dir(), &mut files);
        assert!(!files.is_empty(), "the scan read no file");
        let mut found = Vec::new();
        for path in files {
            let source = fs::read_to_string(&path).unwrap();
            for (index, line) in source.lines().enumerate() {
                if line.contains(MARKER) {
                    found.push(format!("{}:{}", path.display(), index + 1));
                }
            }
        }
        assert!(
            found.is_empty(),
            "transitional shims remain:\n{}",
            found.join("\n")
        );
    }

    /// Every file of a directory, read by a recursive walk.
    fn module_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                module_files(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    #[test]
    fn no_integration_folder_references_another() {
        let root = src_dir().join("integrations");
        let mut checked = 0;
        let modules = integration_modules();
        for folder in &modules {
            let mut files = Vec::new();
            module_files(&root.join(folder), &mut files);
            assert!(!files.is_empty(), "{folder} has no file");
            for path in files {
                let source = fs::read_to_string(&path).unwrap();
                checked += 1;
                for other in &modules {
                    if other == folder {
                        continue;
                    }
                    for line in source.lines().filter(|line| !is_comment(line)) {
                        assert!(
                            !line.contains(&format!("integrations::{other}"))
                                && !line.contains(&format!("super::{other}")),
                            "{} references the {other} module: {line}",
                            path.display()
                        );
                    }
                }
            }
        }
        assert!(checked > 0);
    }

    #[test]
    fn new_identifiers_contain_no_forbidden_surface() {
        // The substrings the scope boundary forbids in core sources.
        const FORBIDDEN: &[&str] = &["codex", "gemini", "copilot", "aider", "acp", "chat ui"];
        const KEYWORDS: [&str; 7] = [
            "fn ", "struct ", "enum ", "trait ", "const ", "static ", "type ",
        ];
        let mut files = Vec::new();
        module_files(&src_dir().join("integrations"), &mut files);
        assert!(!files.is_empty(), "the scan read no file");
        for path in files {
            let source = fs::read_to_string(&path).unwrap();
            for line in source.lines() {
                let declares = KEYWORDS
                    .iter()
                    .any(|keyword| line.trim_start().contains(keyword));
                if !declares || is_comment(line) {
                    continue;
                }
                let lowered = line.to_ascii_lowercase();
                for forbidden in FORBIDDEN {
                    assert!(
                        !lowered.contains(forbidden),
                        "{} declares a forbidden surface `{forbidden}`: {line}",
                        path.display()
                    );
                }
            }
        }
    }

    #[test]
    fn no_dynamic_loading_crate_is_a_direct_dependency_of_the_workspace() {
        const LOADERS: &[&str] = &[
            "libloading",
            "dlopen",
            "wasmtime",
            "wasmer",
            "wasmi",
            "mlua",
            "rhai",
            "rquickjs",
            "deno_core",
            "pyo3",
        ];
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut manifests = vec![workspace.join("Cargo.toml")];
        for entry in fs::read_dir(workspace.join("crates")).unwrap() {
            let manifest = entry.unwrap().path().join("Cargo.toml");
            if manifest.is_file() {
                manifests.push(manifest);
            }
        }
        assert!(manifests.len() > 1, "the scan read no crate manifest");
        for manifest in manifests {
            let text = fs::read_to_string(&manifest).unwrap();
            for line in text.lines() {
                let name = line.split('=').next().unwrap_or("").trim();
                assert!(
                    !LOADERS.contains(&name),
                    "{} depends on {name}",
                    manifest.display()
                );
            }
        }
    }

    #[test]
    fn the_integration_set_is_a_compile_time_list() {
        let keys = |list: made_app::integrations::IntegrationList| -> Vec<&'static str> {
            list.iter().map(|entry| entry.descriptor().key).collect()
        };
        let before = keys(PRODUCT);
        // Naming another agent program in the environment adds nothing: the
        // set is read from no file, variable or directory.
        std::env::set_var("MADE_TEST_AGENT_PROGRAM", "other-agent");
        let after = keys(PRODUCT);
        std::env::remove_var("MADE_TEST_AGENT_PROGRAM");
        assert_eq!(before, after);
        assert_eq!(after, ["opencode", "grok", "claude-code"]);
    }
}
