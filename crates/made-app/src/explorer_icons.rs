//! Explorer file and folder icons from the vendored Material Icon Theme.
//!
//! `build.rs` validates the vendored v5.40.0 mapping and generates the index
//! included below, embedding every SVG with `include_bytes!`. Resolution is a
//! pure lookup over that index: it reads no files and makes no network or npm
//! access. Names fold with [`str::to_lowercase`], which does not depend on the
//! operating-system locale, and the build folds keys the same way.

use made_explorer::tree::EntryKind;

/// One embedded SVG from the vendored theme.
struct Icon {
    /// SVG file name inside `assets/material-icons/icons/`.
    name: &'static str,
    /// Stable `bytes://` URI, so egui caches each icon's texture separately.
    uri: &'static str,
    /// The SVG document, embedded at compile time.
    bytes: &'static [u8],
}

include!(concat!(env!("OUT_DIR"), "/explorer_icons_index.rs"));

/// Handle to one embedded icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IconId(u16);

impl IconId {
    fn icon(self) -> &'static Icon {
        &ICONS[usize::from(self.0)]
    }

    /// SVG file name, for example `rust.svg`.
    pub fn name(self) -> &'static str {
        self.icon().name
    }

    /// `bytes://` URI that keys this icon's texture cache.
    pub fn uri(self) -> &'static str {
        self.icon().uri
    }

    /// Embedded SVG document.
    pub fn bytes(self) -> &'static [u8] {
        self.icon().bytes
    }
}

/// Resolve the theme icon for one Explorer row.
///
/// Depends only on the entry name, its kind, and whether a folder is expanded.
/// Regular files and folders always resolve, falling back to the theme's
/// default icons. Symlink, other, and error rows return `None`, so the caller
/// keeps their explicit `@`, `?`, and `!` markers.
pub fn resolve(name: &str, kind: EntryKind, expanded: bool) -> Option<IconId> {
    match kind {
        EntryKind::File => Some(resolve_file(name)),
        EntryKind::Directory => Some(resolve_folder(name, expanded)),
        EntryKind::Symlink | EntryKind::Other | EntryKind::Error => None,
    }
}

/// Exact file name first, then the longest extension suffix, then the default.
fn resolve_file(name: &str) -> IconId {
    let folded = name.to_lowercase();
    if let Some(id) = lookup(FILE_NAMES, &folded) {
        return id;
    }
    // Dots from left to right yield suffixes from longest to shortest, so
    // `types.d.ts` finds `d.ts` before `ts`.
    for (dot, _) in folded.match_indices('.') {
        let suffix = &folded[dot + 1..];
        if suffix.is_empty() {
            continue;
        }
        if let Some(id) = lookup(FILE_EXTENSIONS, suffix) {
            return id;
        }
    }
    IconId(DEFAULT_FILE)
}

/// Closed or expanded folder variant, falling back to the matching default.
fn resolve_folder(name: &str, expanded: bool) -> IconId {
    let folded = name.to_lowercase();
    if expanded {
        lookup(FOLDER_NAMES_EXPANDED, &folded).unwrap_or(IconId(DEFAULT_FOLDER_EXPANDED))
    } else {
        lookup(FOLDER_NAMES, &folded).unwrap_or(IconId(DEFAULT_FOLDER))
    }
}

fn lookup(table: &'static [(&'static str, u16)], folded: &str) -> Option<IconId> {
    table
        .binary_search_by(|&(key, _)| key.cmp(folded))
        .ok()
        .map(|position| IconId(table[position].1))
}

/// Every embedded icon, in index order.
pub fn icon_ids() -> impl Iterator<Item = IconId> {
    (0..ICONS.len()).map(|index| IconId(u16::try_from(index).expect("index fits in u16")))
}

/// Size and invariants of the generated index, for audits and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexStats {
    /// Embedded SVG icons.
    pub icons: usize,
    /// Exact file-name entries.
    pub file_names: usize,
    /// File-extension entries, including compound suffixes such as `d.ts`.
    pub file_extensions: usize,
    /// Closed folder-name entries.
    pub folder_names: usize,
    /// Expanded folder-name entries.
    pub folder_names_expanded: usize,
    /// Every table is strictly ascending by case-folded key. Binary search is
    /// valid, and no key is duplicated or collides after case folding.
    pub tables_sorted_unique: bool,
}

/// Audit the generated index.
pub fn index_stats() -> IndexStats {
    IndexStats {
        icons: ICONS.len(),
        file_names: FILE_NAMES.len(),
        file_extensions: FILE_EXTENSIONS.len(),
        folder_names: FOLDER_NAMES.len(),
        folder_names_expanded: FOLDER_NAMES_EXPANDED.len(),
        tables_sorted_unique: [
            FILE_NAMES,
            FILE_EXTENSIONS,
            FOLDER_NAMES,
            FOLDER_NAMES_EXPANDED,
        ]
        .into_iter()
        .all(is_sorted_unique_and_folded),
    }
}

fn is_sorted_unique_and_folded(table: &[(&str, u16)]) -> bool {
    table.windows(2).all(|pair| pair[0].0 < pair[1].0)
        && table.iter().all(|&(key, _)| key.to_lowercase() == key)
}
