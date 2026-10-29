//! Explorer file and folder icon assets, resolution, index audit, and
//! rasterization.
//!
//! Everything here runs offline. The tests read only the vendored assets in
//! this crate and the icon index compiled into `made_app`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use made_app::explorer_icons::{self, IconId};
use made_explorer::tree::EntryKind;
use serde_json::Value;

/// Size of the complete Material Icon Theme v5.40.0 SVG set.
const ICON_COUNT: usize = 1256;
const PINNED_TARBALL: &str =
    "https://registry.npmjs.org/material-icon-theme/-/material-icon-theme-5.40.0.tgz";
const PINNED_INTEGRITY: &str =
    "sha512-IrvgusbwtqxhUtVgUy0nToJcfN7CYVRdbBxKMfbo0vab9flVBHA5INZ9UiE5SgRj+PM6opCl5nn5TZdpm2UaxA==";

/// OS metadata files that `build.rs` ignores in `icons/`. This list must stay identical to
/// `OS_METADATA_FILES` in `crates/made-app/build.rs`, so the build and this test agree.
const OS_METADATA_FILES: [&str; 2] = [".DS_Store", "Thumbs.db"];

fn assets_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("material-icons")
}

fn mapping() -> Value {
    let bytes = fs::read(assets_dir().join("material-icons.json")).expect("mapping JSON");
    serde_json::from_slice(&bytes).expect("mapping is valid JSON")
}

/// The vendored SVG file name a definition's `iconPath` names.
fn definition_file(mapping: &Value, definition: &str) -> String {
    let path = mapping["iconDefinitions"][definition]["iconPath"]
        .as_str()
        .unwrap_or_else(|| panic!("{definition} is not a vendored definition"));
    path.strip_prefix("./../icons/")
        .unwrap_or_else(|| panic!("{definition}: {path} does not resolve inside icons/"))
        .to_owned()
}

fn file(name: &str) -> IconId {
    explorer_icons::resolve(name, EntryKind::File, false)
        .unwrap_or_else(|| panic!("{name} must resolve to a file icon"))
}

fn folder(name: &str, expanded: bool) -> IconId {
    explorer_icons::resolve(name, EntryKind::Directory, expanded)
        .unwrap_or_else(|| panic!("{name} must resolve to a folder icon"))
}

#[test]
fn vendored_assets_match_pinned_release_inventory() {
    let root = assets_dir();

    // The asset root holds exactly the mapping, the license, the provenance
    // note, and the icons directory, apart from the same OS metadata files that
    // are exempt directly in `icons/`. No npm module, types, or dev files.
    let expected_root: BTreeSet<String> = ["LICENSE", "README.md", "icons", "material-icons.json"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let mut root_names = BTreeSet::new();
    for entry in fs::read_dir(&root).expect("asset root") {
        let entry = entry.expect("root entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        let file_type = entry.file_type().expect("root file type");
        if file_type.is_file() && OS_METADATA_FILES.contains(&name.as_str()) {
            continue;
        }
        root_names.insert(name);
    }
    assert_eq!(
        root_names, expected_root,
        "the asset root must hold only the vendored release"
    );

    // icons/ holds exactly the complete set of SVG files, apart from the OS metadata
    // files that build.rs ignores. Only those exact regular files are exempt below.
    let mut svg_names = BTreeSet::new();
    for entry in fs::read_dir(root.join("icons")).expect("icons directory") {
        let entry = entry.expect("icon entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        let file_type = entry.file_type().expect("icon file type");
        if file_type.is_file() && OS_METADATA_FILES.contains(&name.as_str()) {
            continue;
        }
        assert!(file_type.is_file(), "{name} must be a regular file");
        assert!(name.ends_with(".svg"), "{name} is not an SVG icon");
        svg_names.insert(name);
    }
    assert_eq!(svg_names.len(), ICON_COUNT, "icons/ must hold 1,256 SVGs");

    // Every mapping icon path normalizes into icons/, and every SVG is used.
    let mapping = mapping();
    let definitions = mapping["iconDefinitions"]
        .as_object()
        .expect("iconDefinitions object");
    assert_eq!(definitions.len(), ICON_COUNT, "iconDefinitions count");
    let mut referenced = BTreeSet::new();
    for key in definitions.keys() {
        let name = definition_file(&mapping, key);
        assert!(
            !name.contains('/') && !name.contains('\\') && name.ends_with(".svg"),
            "{key}: {name} does not name a file in icons/"
        );
        assert!(
            svg_names.contains(&name),
            "{key} references missing icon {name}"
        );
        referenced.insert(name);
    }
    assert_eq!(
        referenced, svg_names,
        "every vendored SVG must be referenced by the mapping"
    );

    // MIT license text and attribution to the upstream project.
    let license = fs::read_to_string(root.join("LICENSE")).expect("LICENSE");
    assert!(license.starts_with("The MIT License (MIT)"));
    assert!(license.contains("Permission is hereby granted, free of charge"));
    assert!(license.contains("Material Extensions"));

    // Pinned provenance, complete-set invariant, and update procedure.
    let readme = fs::read_to_string(root.join("README.md")).expect("README.md");
    for required in [
        "Material Icon Theme",
        "https://github.com/material-extensions/vscode-material-icon-theme",
        "5.40.0",
        PINNED_TARBALL,
        PINNED_INTEGRITY,
        "1,256",
        "Complete-set invariant",
        "Update procedure",
    ] {
        assert!(
            readme.contains(required),
            "README.md is missing {required:?}"
        );
    }
}

#[test]
fn file_icon_resolves_known_extension() {
    assert_eq!(file("main.rs").name(), "rust.svg");
    assert_eq!(file("app.ts").name(), "typescript.svg");
}

#[test]
fn file_icon_prefers_exact_name_over_extension() {
    // `package.json` is an exact file name; `notes.json` matches only `.json`.
    assert_eq!(file("package.json").name(), "nodejs.svg");
    assert_eq!(file("notes.json").name(), "json.svg");
    assert_eq!(file(".gitignore").name(), "git.svg");
}

#[test]
fn file_icon_prefers_longest_compound_extension() {
    assert_eq!(file("types.d.ts").name(), "typescript-def.svg");
    assert_eq!(file("app.ts").name(), "typescript.svg");
}

#[test]
fn file_name_extension_and_folder_matching_are_case_insensitive() {
    assert_eq!(file("PACKAGE.JSON"), file("package.json"));
    assert_eq!(file("Dockerfile"), file("dockerfile"));
    assert_eq!(file("Main.RS"), file("main.rs"));
    assert_eq!(file("TYPES.D.TS"), file("types.d.ts"));
    // Non-ASCII extension keys fold with locale-independent Unicode lowercasing.
    assert_eq!(file("Notes.Ц").name(), "tsil.svg");
    assert_eq!(file("Notes.Ц"), file("notes.ц"));
    assert_eq!(folder("SRC", false), folder("src", false));
    assert_eq!(folder("SRC", true), folder("src", true));
    assert_eq!(folder(".GITHUB", true), folder(".github", true));
}

#[test]
fn unknown_file_uses_default_file_icon() {
    assert_eq!(file("zz-unmapped.xyzzy").name(), "file.svg");
    assert_eq!(file("zz-unmapped").name(), "file.svg");
}

#[test]
fn named_folder_uses_closed_and_open_variants() {
    assert_eq!(folder("src", false).name(), "folder-src.svg");
    assert_eq!(folder("src", true).name(), "folder-src-open.svg");
}

#[test]
fn default_folder_uses_closed_and_open_icons() {
    assert_eq!(folder("zz-unmapped", false).name(), "folder.svg");
    assert_eq!(folder("zz-unmapped", true).name(), "folder-open.svg");
}

#[test]
fn special_rows_do_not_resolve_theme_icons() {
    for kind in [EntryKind::Symlink, EntryKind::Other, EntryKind::Error] {
        assert_eq!(
            explorer_icons::resolve("main.rs", kind, false),
            None,
            "{kind:?} rows keep their explicit marker"
        );
        assert_eq!(
            explorer_icons::resolve("src", kind, true),
            None,
            "{kind:?} rows keep their explicit marker"
        );
    }
}

#[test]
fn icons_resolve_from_embedded_bytes() {
    // Compile-time references to the vendored files. The runtime lookup must
    // return these exact bytes; nothing is read from disk while resolving.
    let rust: &[u8] = include_bytes!("../assets/material-icons/icons/rust.svg");
    let folder_open: &[u8] = include_bytes!("../assets/material-icons/icons/folder-src-open.svg");
    let fallback: &[u8] = include_bytes!("../assets/material-icons/icons/file.svg");
    let cases = [
        (file("main.rs"), "rust.svg", rust),
        (folder("src", true), "folder-src-open.svg", folder_open),
        (file("zz-unmapped"), "file.svg", fallback),
    ];
    for (id, name, expected) in cases {
        assert_eq!(id.name(), name);
        assert_eq!(id.uri(), format!("bytes://material-icons/{name}"));
        let svg = std::str::from_utf8(id.bytes()).expect("embedded SVG is UTF-8");
        assert!(svg.contains("<svg"), "{name} is not an SVG document");
        assert_eq!(
            id.bytes(),
            expected,
            "{name} must resolve to its embedded bytes"
        );
    }
}

#[test]
fn generated_index_is_sorted_and_complete() {
    let mapping = mapping();
    let stats = explorer_icons::index_stats();
    assert_eq!(stats.icons, ICON_COUNT);
    assert_eq!(stats.file_names, 2156);
    assert_eq!(stats.file_extensions, 1391);
    assert_eq!(stats.folder_names, 4674);
    assert_eq!(stats.folder_names_expanded, 4674);
    assert!(
        stats.tables_sorted_unique,
        "every table must be strictly sorted by its case-folded key"
    );

    // Every icon id exposes non-empty embedded bytes.
    let ids: Vec<IconId> = explorer_icons::icon_ids().collect();
    assert_eq!(ids.len(), ICON_COUNT);
    for id in &ids {
        assert!(
            !id.bytes().is_empty(),
            "{} has no embedded bytes",
            id.name()
        );
    }

    // No table has a duplicate or case-fold colliding key in the mapping.
    for table in [
        "fileNames",
        "fileExtensions",
        "folderNames",
        "folderNamesExpanded",
    ] {
        let keys = mapping[table].as_object().expect("table object");
        let folded: BTreeSet<String> = keys.keys().map(|key| key.to_lowercase()).collect();
        assert_eq!(folded.len(), keys.len(), "{table} has case-fold collisions");
    }

    // Every key maps to the vendored file its definition's iconPath names.
    for (key, target) in mapping["fileNames"].as_object().expect("fileNames") {
        let target = target.as_str().expect("definition key");
        assert_eq!(
            file(key).name(),
            definition_file(&mapping, target),
            "fileNames {key}"
        );
    }
    for (key, target) in mapping["fileExtensions"]
        .as_object()
        .expect("fileExtensions")
    {
        let target = target.as_str().expect("definition key");
        // The probe is not a known file name, so only the extension can match.
        assert_eq!(
            file(&format!("zz-probe.{key}")).name(),
            definition_file(&mapping, target),
            "fileExtensions {key}"
        );
    }
    for (key, target) in mapping["folderNames"].as_object().expect("folderNames") {
        let target = target.as_str().expect("definition key");
        assert_eq!(
            folder(key, false).name(),
            definition_file(&mapping, target),
            "folderNames {key}"
        );
    }
    for (key, target) in mapping["folderNamesExpanded"]
        .as_object()
        .expect("folderNamesExpanded")
    {
        let target = target.as_str().expect("definition key");
        assert_eq!(
            folder(key, true).name(),
            definition_file(&mapping, target),
            "folderNamesExpanded {key}"
        );
    }
}

/// Rasterize one embedded icon through the app's theme pipeline at 32px.
fn rasterize(ctx: &eframe::egui::Context, id: IconId) -> Arc<eframe::egui::ColorImage> {
    ctx.include_bytes(id.uri(), id.bytes());
    let size_hint = eframe::egui::load::SizeHint::Size {
        width: 32,
        height: 32,
        maintain_aspect_ratio: false,
    };
    match ctx.try_load_image(id.uri(), size_hint) {
        Ok(eframe::egui::load::ImagePoll::Ready { image }) => image,
        Ok(eframe::egui::load::ImagePoll::Pending { .. }) => {
            panic!("{} is still pending", id.name())
        }
        Err(error) => panic!("{} did not rasterize: {error:?}", id.name()),
    }
}

/// Distinct colors of the visible (non-transparent) pixels.
fn visible_colors(image: &eframe::egui::ColorImage) -> BTreeSet<[u8; 4]> {
    image
        .pixels
        .iter()
        .filter(|pixel| pixel.a() > 0)
        .map(|pixel| pixel.to_array())
        .collect()
}

#[test]
fn all_vendored_icons_rasterize_to_visible_pixels() {
    let ctx = eframe::egui::Context::default();
    made_app::theme::install(&ctx);
    for id in explorer_icons::icon_ids() {
        let image = rasterize(&ctx, id);
        assert!(
            image.size[0] > 0 && image.size[1] > 0,
            "{} rasterized to an empty image",
            id.name()
        );
        assert!(
            !visible_colors(&image).is_empty(),
            "{} rasterized fully transparent",
            id.name()
        );
    }
}

#[test]
fn known_icon_keeps_multiple_colors() {
    let ctx = eframe::egui::Context::default();
    made_app::theme::install(&ctx);
    let typescript = explorer_icons::icon_ids()
        .find(|id| id.name() == "typescript.svg")
        .expect("typescript.svg is vendored");
    let colors = visible_colors(&rasterize(&ctx, typescript));
    assert!(
        colors.len() > 1,
        "typescript.svg must keep its distinct artwork colors, found {}",
        colors.len()
    );
}
