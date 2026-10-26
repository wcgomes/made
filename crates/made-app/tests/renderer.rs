//! Renderer tests: cell metrics, grouped runs, truecolor/style mapping,
//! selection, resize, and damage/version rebuilding. No GPU or display is used.

use eframe::egui::vec2;
use made_app::render::{
    build_row_runs, CellMetrics, CellPoint, GridPalette, GridRenderer, SelectionRange,
};
use made_terminal::{
    CellSnapshot, CursorSnapshot, DamageLine, TerminalColor, TerminalCore, TerminalDamage,
    TerminalSize, TerminalSnapshot,
};

fn cell(ch: char, fg: TerminalColor, bg: TerminalColor) -> CellSnapshot {
    CellSnapshot {
        ch,
        fg,
        bg,
        ..CellSnapshot::default()
    }
}

fn snapshot(version: u64, rows: usize, cols: usize, cells: Vec<CellSnapshot>) -> TerminalSnapshot {
    TerminalSnapshot {
        size: TerminalSize { rows, cols },
        cursor: CursorSnapshot::default(),
        cells,
        alternate_screen: false,
        title: Some("renderer test".to_owned()),
        cwd: None,
        display_offset: 0,
        version,
    }
}

#[test]
fn cell_metrics_convert_between_pixels_and_cells() {
    let metrics = CellMetrics::new(8.0, 16.0);
    assert_eq!(
        metrics.grid_size(vec2(80.0, 48.0)),
        TerminalSize { rows: 3, cols: 10 }
    );
    assert_eq!(
        metrics.pixel_size(TerminalSize { rows: 3, cols: 10 }),
        vec2(80.0, 48.0)
    );

    // Degenerate measurements are clamped instead of producing a zero-width
    // grid that would divide by zero in the renderer.
    let clamped = CellMetrics::new(0.0, f32::NAN);
    assert!(clamped.width >= 1.0);
    assert!(clamped.height >= 1.0);
}

#[test]
fn contiguous_cells_group_and_truecolor_maps_to_the_run() {
    let palette = GridPalette::default();
    let metrics = CellMetrics::new(8.0, 16.0);
    let cells = vec![
        cell('a', TerminalColor::Default, TerminalColor::Default),
        cell('b', TerminalColor::Default, TerminalColor::Default),
        cell(
            'c',
            TerminalColor::Rgb(1, 2, 3),
            TerminalColor::Rgb(4, 5, 6),
        ),
    ];
    let snap = snapshot(1, 1, 3, cells);
    let mut renderer = GridRenderer::new(metrics);
    let rows = renderer.frame(
        &snap,
        &TerminalDamage {
            full: true,
            lines: Vec::new(),
        },
        None,
        &palette,
    );

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].runs.len(), 2);
    assert_eq!(rows[0].runs[0].text, "ab");
    assert_eq!(rows[0].runs[0].cells, 2);
    assert_eq!(rows[0].runs[1].text, "c");
    assert_eq!(rows[0].runs[1].style.foreground, [1, 2, 3]);
    assert_eq!(rows[0].runs[1].style.background, [4, 5, 6]);
    assert_eq!(rows[0].covered_cells(), 3);
}

#[test]
fn inverse_and_selection_styles_are_visible_in_runs() {
    let palette = GridPalette::default();
    let mut inverse = cell(
        'x',
        TerminalColor::Rgb(10, 20, 30),
        TerminalColor::Rgb(40, 50, 60),
    );
    inverse.attributes.inverse = true;
    let runs = build_row_runs(0, &[inverse], None, &palette);
    assert_eq!(runs[0].style.foreground, [40, 50, 60]);
    assert_eq!(runs[0].style.background, [10, 20, 30]);

    let cells = vec![
        cell('a', TerminalColor::Default, TerminalColor::Default),
        cell('b', TerminalColor::Default, TerminalColor::Default),
        cell('c', TerminalColor::Default, TerminalColor::Default),
    ];
    let selection = SelectionRange::new(CellPoint::new(0, 1), CellPoint::new(0, 1));
    let runs = build_row_runs(0, &cells, Some(selection), &palette);
    assert_eq!(runs.len(), 3);
    assert!(runs[1].style.selected);
    assert_eq!(runs[1].style.background, palette.selection_background);
    assert_eq!(runs[1].style.foreground, palette.selection_foreground);
}

#[test]
fn wide_glyph_and_spacer_become_a_two_cell_run() {
    let palette = GridPalette::default();
    let mut wide = cell('界', TerminalColor::Default, TerminalColor::Default);
    wide.wide = true;
    let spacer = CellSnapshot {
        wide_spacer: true,
        ..CellSnapshot::default()
    };
    let runs = build_row_runs(
        0,
        &[
            cell('a', TerminalColor::Default, TerminalColor::Default),
            wide,
            spacer,
            cell('b', TerminalColor::Default, TerminalColor::Default),
        ],
        None,
        &palette,
    );
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[1].text, "界");
    assert_eq!(runs[1].cells, 2);
    assert!(runs[1].wide);
}

#[test]
fn damage_and_version_only_rebuild_affected_rows() {
    let palette = GridPalette::default();
    let metrics = CellMetrics::new(8.0, 16.0);
    let cells = vec![
        cell('a', TerminalColor::Default, TerminalColor::Default),
        cell('b', TerminalColor::Default, TerminalColor::Default),
        cell('c', TerminalColor::Default, TerminalColor::Default),
        cell('d', TerminalColor::Default, TerminalColor::Default),
    ];
    let first = snapshot(1, 2, 2, cells.clone());
    let mut renderer = GridRenderer::new(metrics);

    let rows = renderer
        .frame(
            &first,
            &TerminalDamage {
                full: true,
                lines: Vec::new(),
            },
            None,
            &palette,
        )
        .to_vec();
    let original_row_zero = rows[0].clone();
    assert!(renderer.last_rebuild_was_full());
    assert_eq!(renderer.last_rebuild_rows(), 2);

    // Same version and no damage: no work.
    let rows = renderer
        .frame(&first, &TerminalDamage::default(), None, &palette)
        .to_vec();
    assert_eq!(renderer.last_rebuild_rows(), 0);
    assert_eq!(rows[0], original_row_zero);

    // New version, one damaged line: only row one is rebuilt.
    let mut second = first.clone();
    second.version = 2;
    second.cells[3] = cell('z', TerminalColor::Default, TerminalColor::Default);
    let rows = renderer
        .frame(
            &second,
            &TerminalDamage {
                full: false,
                lines: vec![DamageLine {
                    row: 1,
                    left: 1,
                    right: 1,
                }],
            },
            None,
            &palette,
        )
        .to_vec();
    assert_eq!(renderer.last_rebuild_rows(), 1);
    assert!(!renderer.last_rebuild_was_full());
    assert_eq!(rows[0], original_row_zero);
    assert_eq!(rows[1].runs[0].text, "cz");

    // A selection change dirties only the selected row.
    let selection = SelectionRange::new(CellPoint::new(0, 0), CellPoint::new(0, 1));
    renderer.frame(
        &second,
        &TerminalDamage::default(),
        Some(selection),
        &palette,
    );
    assert_eq!(renderer.last_rebuild_rows(), 1);
    assert!(renderer.row(0).unwrap().runs[0].style.selected);
}

#[test]
fn resize_rebuilds_the_whole_viewport() {
    let palette = GridPalette::default();
    let metrics = CellMetrics::new(8.0, 16.0);
    let mut renderer = GridRenderer::new(metrics);
    let first = snapshot(
        1,
        1,
        1,
        vec![cell('a', TerminalColor::Default, TerminalColor::Default)],
    );
    renderer.frame(
        &first,
        &TerminalDamage {
            full: true,
            lines: Vec::new(),
        },
        None,
        &palette,
    );

    let resized = snapshot(
        2,
        2,
        3,
        vec![cell('x', TerminalColor::Default, TerminalColor::Default); 6],
    );
    let rows = renderer.frame(
        &resized,
        &TerminalDamage {
            full: true,
            lines: Vec::new(),
        },
        None,
        &palette,
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(renderer.size(), TerminalSize { rows: 2, cols: 3 });
    assert!(renderer.last_rebuild_was_full());
    assert_eq!(renderer.last_rebuild_rows(), 2);
}

#[test]
fn terminal_core_escape_sequences_reach_truecolor_runs() {
    let palette = GridPalette::default();
    let mut core = TerminalCore::with_size(TerminalSize { rows: 2, cols: 8 }, 100);
    core.feed(b"\x1b[38;2;12;34;56mT\x1b[0m");
    let snap = core.snapshot();
    let damage = core.take_damage();
    let mut renderer = GridRenderer::new(CellMetrics::new(8.0, 16.0));
    let rows = renderer.frame(&snap, &damage, None, &palette);
    assert_eq!(rows[0].runs[0].text, "T");
    assert_eq!(rows[0].runs[0].style.foreground, [12, 34, 56]);
}

#[test]
fn hidden_cells_keep_the_column_of_the_following_visible_cell() {
    let palette = GridPalette::default();
    let mut hidden = cell('x', TerminalColor::Default, TerminalColor::Default);
    hidden.attributes.hidden = true;
    let runs = build_row_runs(
        0,
        &[
            hidden,
            cell('a', TerminalColor::Default, TerminalColor::Default),
        ],
        None,
        &palette,
    );
    assert_eq!(runs.len(), 2);
    assert!(runs[0].text.is_empty());
    assert_eq!(runs[1].start_col, 1);
    assert_eq!(runs[1].text, "a");
}

#[test]
fn palette_maps_ansi_cube_and_grayscale() {
    let palette = GridPalette::default();
    assert_eq!(palette.indexed(1), palette.ansi[1]);
    assert_eq!(palette.indexed(16), [0, 0, 0]);
    assert_eq!(palette.indexed(231), [255, 255, 255]);
    assert_eq!(palette.indexed(244), [128, 128, 128]);
}
