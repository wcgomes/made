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
    /// name them outside the exempt regions.
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
    /// them outside the exempt regions.
    const LEGACY_GROK: &[&str] = &[
        "grok_root",
        "GrokLiveSession",
        "poll_grok_",
        "GrokDiagnosticsPhase",
        "grok_row_for",
    ];

    /// Identifiers of the legacy OpenCode seams. Core files must not name
    /// them outside the exempt regions.
    const LEGACY_OPENCODE: &[&str] = &[
        "workspace_opencode_active",
        "opencode_icon",
        "remember_opencode_missing",
        "note_opencode_",
    ];

    /// Whole files that are remote paths.
    const REMOTE_FILES: &[&str] = &["ssh_servers.rs", "remote_workspace.rs", "remote_session.rs"];

    const MODULES: [&str; 3] = ["claude_code", "grok", "opencode"];

    fn src_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
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

    /// The lines of `source` outside remote-path and transitional-shim
    /// regions, with their numbers.
    fn scanned_lines(source: &str) -> Vec<(usize, &str)> {
        let mut depth = 0usize;
        let mut lines = Vec::new();
        for (index, line) in source.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("// BEGIN remote-path")
                || trimmed.starts_with("// BEGIN transitional-shim")
            {
                depth += 1;
                continue;
            }
            if trimmed.starts_with("// END remote-path")
                || trimmed.starts_with("// END transitional-shim")
            {
                depth = depth.saturating_sub(1);
                continue;
            }
            if depth == 0 {
                lines.push((index + 1, line));
            }
        }
        lines
    }

    /// The part of a core file the scans read: everything before its unit
    /// tests.
    fn production_part(source: &str) -> String {
        let lines: Vec<&str> = source.lines().collect();
        let end = lines
            .windows(2)
            .position(|pair| {
                pair[0].trim_end() == "#[cfg(test)]" && pair[1].starts_with("mod tests")
            })
            .unwrap_or(lines.len());
        lines[..end].join("\n")
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
        let mut found = Vec::new();
        for path in core_files() {
            let source = fs::read_to_string(&path).unwrap();
            for (number, line) in scanned_lines(&production_part(&source)) {
                if is_comment(line) {
                    continue;
                }
                if MODULES
                    .iter()
                    .any(|module| line.contains(&format!("integrations::{module}")))
                {
                    found.push(format!("{}:{number}", path.display()));
                }
            }
        }
        assert!(
            found.is_empty(),
            "core imports of an integration module:\n{}",
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
        for folder in MODULES {
            let mut files = Vec::new();
            module_files(&root.join(folder), &mut files);
            assert!(!files.is_empty(), "{folder} has no file");
            for path in files {
                let source = fs::read_to_string(&path).unwrap();
                checked += 1;
                for other in MODULES {
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
