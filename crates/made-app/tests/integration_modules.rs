//! Pins and validation of the integration list (AIA-03, AIA-05, AIA-09).
//!
//! The pin test asserts every descriptor field of the product list verbatim, so
//! a change to the list or to any identifier trips it. Validation is exercised
//! with lists defined only in this file.

use made_app::integrations::{
    validate, Descriptor, Integration, IntegrationList, ListError, Mark, Rank, CLAUDE_CODE, GROK,
    OPENCODE, PRODUCT,
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
    owns_session_channel: bool,
}

impl Integration for Probe {
    fn descriptor(&self) -> &'static Descriptor {
        self.descriptor
    }

    fn owns_session_channel(&self) -> bool {
        self.owns_session_channel
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
    owns_session_channel: false,
};
static PROBE_SAME_KEY: Probe = Probe {
    descriptor: &SAME_KEY,
    owns_session_channel: false,
};
static PROBE_SAME_RANK: Probe = Probe {
    descriptor: &SAME_RANK,
    owns_session_channel: false,
};
static PROBE_SECOND: Probe = Probe {
    descriptor: &SECOND,
    owns_session_channel: false,
};
static PROBE_THIRD_OWNER: Probe = Probe {
    descriptor: &THIRD,
    owns_session_channel: true,
};
static PROBE_FIRST_OWNER: Probe = Probe {
    descriptor: &FIRST,
    owns_session_channel: true,
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
