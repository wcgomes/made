//! Provenance and licenses of the bundled terminal fonts.
//!
//! Runs offline against the vendored files in `assets/terminal-font/`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const DEJAVU_ASSET: &str =
    "https://github.com/dejavu-fonts/dejavu-fonts/releases/download/version_2_37/dejavu-fonts-ttf-2.37.zip";
const DEJAVU_SHA256: &str = "7576310b219e04159d35ff61dd4a4ec4cdba4f35c00e002a136f00e96a908b0a";
const IOSEVKA_ASSET: &str =
    "https://github.com/be5invis/Iosevka/releases/download/v34.9.1/PkgTTF-IosevkaTerm-34.9.1.zip";
const IOSEVKA_SHA256: &str = "3bd57a66a7f1e429a131d3bc35f4d0f3a8eb722f2fe8f59f595e424bda9a33c9";
/// OS metadata files ignored in the asset root, as in `explorer_icons.rs`.
const OS_METADATA_FILES: [&str; 2] = [".DS_Store", "Thumbs.db"];

fn assets_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("terminal-font")
}

#[test]
fn terminal_font_assets_hold_the_subsets_licenses_and_provenance() {
    let root = assets_dir();

    // Exactly the two subsets, their licenses, and the provenance note, apart
    // from OS metadata files.
    let expected: BTreeSet<String> = [
        "DejaVuSansMono.ttf",
        "IosevkaTermExtended-Symbols.ttf",
        "LICENSE-DejaVu",
        "LICENSE-Iosevka",
        "README.md",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(&root).expect("asset root") {
        let entry = entry.expect("asset entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if OS_METADATA_FILES.contains(&name.as_str()) {
            continue;
        }
        assert!(
            entry.file_type().expect("file type").is_file(),
            "{name} must be a regular file"
        );
        names.insert(name);
    }
    assert_eq!(names, expected, "the asset root must hold only the subset");

    // DejaVu: the Bitstream Vera license with the public-domain DejaVu changes.
    let dejavu = fs::read_to_string(root.join("LICENSE-DejaVu")).expect("LICENSE-DejaVu");
    assert!(dejavu
        .starts_with("Fonts are (c) Bitstream (see below). DejaVu changes are in public domain."));
    assert!(dejavu.contains("Copyright (c) 2003 by Bitstream, Inc. All Rights Reserved."));
    assert!(dejavu.contains("Bitstream Vera Fonts Copyright"));

    // Iosevka: OFL 1.1 text with the upstream copyright line.
    let iosevka = fs::read_to_string(root.join("LICENSE-Iosevka")).expect("LICENSE-Iosevka");
    assert!(iosevka.contains("SIL Open Font License"));
    assert!(iosevka.contains("Version 1.1"));
    assert!(iosevka
        .contains("Copyright (c) 2015-2026, Renzhi Li (aka. Belleve Invis, belleve@typeof.net)"));

    // Pinned source releases and update procedure.
    let readme = fs::read_to_string(root.join("README.md")).expect("README.md");
    for required in [
        "https://github.com/dejavu-fonts/dejavu-fonts",
        "2.37",
        DEJAVU_ASSET,
        DEJAVU_SHA256,
        "https://github.com/be5invis/Iosevka",
        "34.9.1",
        IOSEVKA_ASSET,
        IOSEVKA_SHA256,
        "scripts/subset-terminal-fonts.sh",
        "Update procedure",
    ] {
        assert!(
            readme.contains(required),
            "README.md is missing {required:?}"
        );
    }
}
