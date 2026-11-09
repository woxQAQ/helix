use super::*;

use arc_swap::ArcSwap;
use helix_core::doc_formatter::DocumentFormatter;
use helix_term::{
    compositor::{Compositor, Context, Event},
    job::Jobs,
    keymap::{self, Keymaps},
    ui::EditorView,
};
use helix_view::{
    current_ref,
    graphics::{Modifier, Rect},
    input::{parse_macro, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    view::ViewPosition,
};
use tui::buffer::Buffer;

#[tokio::test(flavor = "multi_thread")]
async fn folding_marker_is_visible_in_line_numbers() -> anyhow::Result<()> {
    for soft_wrap in [false, true] {
        let mut config = test_config();
        config.editor.soft_wrap.enable = Some(soft_wrap);
        let mut app = AppBuilder::new()
            .with_config(config)
            .with_input_text("#[head\nhidden 中文\nhidden two|]#\ntail\n")
            .build()?;
        let original = current_ref!(app.editor).1.text().to_string();
        let area = Rect::new(3, 2, 60, 15);
        let mut compositor = Compositor::new(area);
        compositor.push(Box::new(EditorView::new(Keymaps::new(Box::new(
            ArcSwap::from_pointee(keymap::default()),
        )))));
        let mut jobs = Jobs::new();
        let mut cx = Context {
            editor: &mut app.editor,
            scroll: None,
            jobs: &mut jobs,
        };
        let mut surface = Buffer::empty(area);
        compositor.render(area, &mut surface, &mut cx);
        for key in parse_macro("zf")? {
            compositor.handle_event(&Event::Key(key), &mut cx);
        }
        surface.reset();
        compositor.render(area, &mut surface, &mut cx);
        let (view, doc) = current_ref!(cx.editor);
        let inner = view.inner_area(doc);
        let rows: Vec<String> = (inner.y..inner.bottom())
            .map(|y| {
                (inner.x..inner.right())
                    .map(|x| surface[(x, y)].symbol.as_str())
                    .collect()
            })
            .collect();
        assert_eq!(rows[0].trim_end(), "head…");
        assert_eq!(rows[1].trim_end(), "tail");
        assert!(rows.iter().all(|row| !row.contains("hidden")));
        assert_eq!(doc.text().to_string(), original);
        let chevron = view.fold_toggle_column(doc).unwrap();
        assert_eq!(surface[(chevron, inner.y)].symbol.as_str(), "▶");
        assert_eq!(surface[(chevron - 1, inner.y)].symbol.as_str(), " ");
        assert!(surface[(chevron, inner.y)]
            .modifier
            .contains(Modifier::BOLD));
        assert_eq!(
            Some(surface[(chevron, inner.y)].fg),
            cx.editor.theme.get("ui.text").fg
        );
        assert_eq!(surface[(chevron, inner.y + 1)].symbol.as_str(), " ");
        for key in parse_macro("zo")? {
            compositor.handle_event(&Event::Key(key), &mut cx);
        }
        surface.reset();
        compositor.render(area, &mut surface, &mut cx);
        let (view, doc) = current_ref!(cx.editor);
        let inner = view.inner_area(doc);
        let rows: Vec<String> = (inner.y..inner.bottom())
            .map(|y| {
                (inner.x..inner.right())
                    .map(|x| surface[(x, y)].symbol.as_str())
                    .collect()
            })
            .collect();
        assert_eq!(rows[0].trim_end(), "head");
        assert!(rows[1].contains("hidden"));
        assert!(rows[2].contains("hidden two"));
        assert_eq!(rows[3].trim_end(), "tail");
        assert_eq!(
            surface[(view.fold_toggle_column(doc).unwrap(), inner.y)]
                .symbol
                .as_str(),
            " "
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn folding_preserves_highlights_and_chevrons_toggle_with_mouse() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("highlight.rs");
    let tail = "fn after() { let value = (1 + 2); }";
    let source = format!(
        "fn folded() {{\n{}}}\n{tail}\n",
        "    let hidden = \"中文\";\n".repeat(80)
    );
    std::fs::write(&path, &source)?;
    let mut config = test_config();
    config.editor.rainbow_brackets = true;
    let mut app = AppBuilder::new()
        .with_config(config)
        .with_file(&path, None)
        .build()?;
    let area = Rect::new(3, 2, 80, 15);
    let mut compositor = Compositor::new(area);
    compositor.push(Box::new(EditorView::new(Keymaps::new(Box::new(
        ArcSwap::from_pointee(keymap::default()),
    )))));
    let mut jobs = Jobs::new();
    let mut cx = Context {
        editor: &mut app.editor,
        scroll: None,
        jobs: &mut jobs,
    };
    let mut surface = Buffer::empty(area);
    compositor.render(area, &mut surface, &mut cx);
    let (view, doc) = current_ref!(cx.editor);
    let chevron = view.fold_toggle_column(doc).unwrap();
    let inner = view.inner_area(doc);
    assert_eq!(surface[(chevron, inner.y)].symbol.as_str(), "▼");
    {
        let (view, doc) = helix_view::current!(cx.editor);
        let anchor = doc.text().line_to_char(82);
        doc.set_view_offset(
            view.id,
            ViewPosition {
                anchor,
                horizontal_offset: 0,
                vertical_offset: 0,
            },
        );
    }
    surface.reset();
    compositor.render(area, &mut surface, &mut cx);
    let expected: Vec<_> = (0..tail.len() as u16)
        .map(|x| surface[(inner.x + x, inner.y)].clone())
        .collect();
    assert!(
        expected.iter().any(|cell| cell.fg != expected[0].fg),
        "reference line must have syntax highlighting"
    );
    {
        let (view, doc) = helix_view::current!(cx.editor);
        doc.set_view_offset(
            view.id,
            ViewPosition {
                anchor: 0,
                horizontal_offset: 0,
                vertical_offset: 0,
            },
        );
    }
    surface.reset();
    compositor.render(area, &mut surface, &mut cx);
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        compositor.handle_event(
            &Event::Mouse(MouseEvent {
                kind,
                column: chevron,
                row: inner.y,
                modifiers: KeyModifiers::empty(),
            }),
            &mut cx,
        );
    }
    surface.reset();
    compositor.render(area, &mut surface, &mut cx);
    let (view, doc) = current_ref!(cx.editor);
    assert_eq!(doc.folds(view.id).len(), 1);
    assert_eq!(surface[(chevron, inner.y)].symbol.as_str(), "▶");
    let actual: Vec<_> = (0..tail.len() as u16)
        .map(|x| surface[(inner.x + x, inner.y + 1)].clone())
        .collect();
    assert_eq!(
        actual, expected,
        "highlight styles after a long fold must match the unfolded viewport"
    );
    assert_eq!(doc.text().to_string(), source);
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        compositor.handle_event(
            &Event::Mouse(MouseEvent {
                kind,
                column: chevron,
                row: inner.y,
                modifiers: KeyModifiers::empty(),
            }),
            &mut cx,
        );
    }
    surface.reset();
    compositor.render(area, &mut surface, &mut cx);
    let (view, doc) = current_ref!(cx.editor);
    assert!(doc.folds(view.id).is_empty());
    assert_eq!(surface[(chevron, inner.y)].symbol.as_str(), "▼");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn folding_match_preserves_closing_delimiters_and_navigation() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("match.rs");
    for soft_wrap in [false, true] {
        for newline in ["\n", "\r\n"] {
            let source = [
                "fn example(&self) {",
                "    match self {",
                "        Self::A => \"中文\",",
                "        Self::B => \"other\",",
                "    };",
                "    let tail = 1;",
                "}",
                "",
            ]
            .join(newline);
            std::fs::write(&path, &source)?;
            let mut config = test_config();
            config.editor.soft_wrap.enable = Some(soft_wrap);
            let mut app = AppBuilder::new()
                .with_config(config)
                .with_file(&path, Some(helix_core::Position::new(1, 4)))
                .build()?;
            let area = Rect::new(3, 2, 80, 15);
            let mut compositor = Compositor::new(area);
            compositor.push(Box::new(EditorView::new(Keymaps::new(Box::new(
                ArcSwap::from_pointee(keymap::default()),
            )))));
            let mut jobs = Jobs::new();
            let mut cx = Context {
                editor: &mut app.editor,
                scroll: None,
                jobs: &mut jobs,
            };
            let mut surface = Buffer::empty(area);
            compositor.render(area, &mut surface, &mut cx);
            for key in parse_macro("zf")? {
                compositor.handle_event(&Event::Key(key), &mut cx);
            }
            surface.reset();
            compositor.render(area, &mut surface, &mut cx);
            let (view, doc) = current_ref!(cx.editor);
            let inner = view.inner_area(doc);
            let rows: Vec<String> = (inner.y..inner.bottom())
                .map(|y| {
                    (inner.x..inner.right())
                        .map(|x| surface[(x, y)].symbol.as_str())
                        .collect()
                })
                .collect();
            assert_eq!(rows[1].trim_end(), "    match self {…};");
            assert_eq!(rows[2].trim_end(), "    let tail = 1;");
            assert_eq!(rows[3].trim_end(), "}");
            assert!(rows.iter().all(|row| !row.contains("Self::")));
            let end = doc.folds(view.id)[0].end;
            assert_eq!(doc.text().char(end), '}');
            let closing_column = inner.x + "    match self {…".chars().count() as u16;
            assert_eq!(
                view.pos_at_screen_coords(doc, inner.y + 1, closing_column, false),
                Some(end)
            );
            for kind in [
                MouseEventKind::Down(MouseButton::Left),
                MouseEventKind::Up(MouseButton::Left),
            ] {
                compositor.handle_event(
                    &Event::Mouse(MouseEvent {
                        kind,
                        row: inner.y + 1,
                        column: closing_column + 1,
                        modifiers: KeyModifiers::empty(),
                    }),
                    &mut cx,
                );
            }
            for key in parse_macro("j")? {
                compositor.handle_event(&Event::Key(key), &mut cx);
            }
            let (view, doc) = current_ref!(cx.editor);
            assert_eq!(
                doc.selection(view.id)
                    .primary()
                    .cursor_line(doc.text().slice(..)),
                5
            );
            assert_eq!(doc.folds(view.id).len(), 1);
            for key in parse_macro("k")? {
                compositor.handle_event(&Event::Key(key), &mut cx);
            }
            let (view, doc) = current_ref!(cx.editor);
            let cursor = doc
                .selection(view.id)
                .primary()
                .cursor(doc.text().slice(..));
            assert_eq!(
                view.screen_coords_at_pos(doc, doc.text().slice(..), cursor)
                    .unwrap()
                    .row,
                1
            );
            assert_eq!(doc.folds(view.id).len(), 1);
            for key in parse_macro("zo")? {
                compositor.handle_event(&Event::Key(key), &mut cx);
            }
            let (view, doc) = current_ref!(cx.editor);
            assert!(doc.folds(view.id).is_empty());
            assert_eq!(doc.text().to_string(), source);
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn folding_renders_and_moves_over_hidden_lines() -> anyhow::Result<()> {
    for soft_wrap in [false, true] {
        for newline in ["\n", "\r\n"] {
            let mut config = test_config();
            config.editor.soft_wrap.enable = Some(soft_wrap);
            let input =
                format!("#[head{newline}hidden 中文{newline}hidden two|]#{newline}tail{newline}");
            let mut app = AppBuilder::new()
                .with_config(config)
                .with_input_text(input)
                .build()?;
            test_key_sequences(
                &mut app,
                vec![
                    (
                        Some("zf"),
                        Some(&|app| {
                            let (view, doc) = current_ref!(app.editor);
                            assert_eq!(doc.folds(view.id).len(), 1);
                            let format = doc.text_format(view.inner_width(doc), None);
                            let annotations = view.text_annotations(doc, None);
                            let graphemes: Vec<_> = DocumentFormatter::new_at_prev_checkpoint(
                                doc.text().slice(..),
                                &format,
                                &annotations,
                                0,
                            )
                            .collect();
                            assert!(graphemes.iter().any(|g| g.doc_chars() > 5));
                            assert!(!graphemes.iter().any(|g| g.line_idx == 1));
                            let tail = doc.text().line_to_char(3);
                            let position = view
                                .screen_coords_at_pos(doc, doc.text().slice(..), tail)
                                .unwrap();
                            assert_eq!(position.row, 1);
                            let inner = view.inner_area(doc);
                            assert_eq!(
                                view.pos_at_screen_coords(doc, inner.y + 1, inner.x, false),
                                Some(tail)
                            );
                        }),
                    ),
                    (
                        Some("gh5l"),
                        Some(&|app| {
                            let (view, doc) = current_ref!(app.editor);
                            assert_eq!(
                                doc.selection(view.id)
                                    .primary()
                                    .cursor(doc.text().slice(..)),
                                doc.folds(view.id)[0].end
                            );
                        }),
                    ),
                    (
                        Some("j"),
                        Some(&|app| {
                            let (view, doc) = current_ref!(app.editor);
                            assert_eq!(
                                doc.selection(view.id)
                                    .primary()
                                    .cursor_line(doc.text().slice(..)),
                                3
                            );
                            assert_eq!(doc.folds(view.id).len(), 1);
                        }),
                    ),
                    (
                        Some("k"),
                        Some(&|app| {
                            let (view, doc) = current_ref!(app.editor);
                            assert_eq!(
                                doc.selection(view.id)
                                    .primary()
                                    .cursor_line(doc.text().slice(..)),
                                0
                            );
                        }),
                    ),
                    (
                        Some("2j2k"),
                        Some(&|app| {
                            let (view, doc) = current_ref!(app.editor);
                            assert_eq!(
                                doc.selection(view.id)
                                    .primary()
                                    .cursor_line(doc.text().slice(..)),
                                0
                            );
                            assert_eq!(doc.folds(view.id).len(), 1);
                        }),
                    ),
                    (
                        Some("gh5lzo"),
                        Some(&|app| {
                            let (view, doc) = current_ref!(app.editor);
                            assert!(doc.folds(view.id).is_empty());
                            let tail = doc.text().line_to_char(3);
                            assert_eq!(
                                view.screen_coords_at_pos(doc, doc.text().slice(..), tail)
                                    .unwrap()
                                    .row,
                                3
                            );
                        }),
                    ),
                ],
                false,
            )
            .await?;
        }
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn folding_syntax_toggle_and_split_views() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("fold.rs");
    std::fs::write(
        &path,
        "fn main() {\n    println!(\"hello\");\n}\n\nfn other() {}\n",
    )?;
    let mut app = AppBuilder::new().with_file(path, None).build()?;
    test_key_sequences(
        &mut app,
        vec![
            (
                Some("zf"),
                Some(&|app| {
                    let (view, doc) = current_ref!(app.editor);
                    assert_eq!(doc.folds(view.id).len(), 1);
                    assert_eq!(doc.text().char_to_line(doc.folds(view.id)[0].end), 2);
                }),
            ),
            (
                Some("za"),
                Some(&|app| {
                    let (view, doc) = current_ref!(app.editor);
                    assert!(doc.folds(view.id).is_empty());
                }),
            ),
            (
                Some("za<C-w>v"),
                Some(&|app| {
                    let (view, doc) = current_ref!(app.editor);
                    assert!(doc.folds(view.id).is_empty());
                }),
            ),
            (
                Some("<C-w>w"),
                Some(&|app| {
                    let (view, doc) = current_ref!(app.editor);
                    assert_eq!(doc.folds(view.id).len(), 1);
                }),
            ),
            (
                Some("zR<C-w>q"),
                Some(&|app| {
                    let (view, doc) = current_ref!(app.editor);
                    assert!(doc.folds(view.id).is_empty());
                }),
            ),
        ],
        false,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn folding_search_reveals_hidden_text_and_edits_map_ranges() -> anyhow::Result<()> {
    let mut app = AppBuilder::new()
        .with_input_text("#[head\nhidden one\nhidden two|]#\ntail\n")
        .build()?;
    test_key_sequences(
        &mut app,
        vec![
            (
                Some("zfOabove<esc>"),
                Some(&|app| {
                    let (view, doc) = current_ref!(app.editor);
                    assert_eq!(doc.folds(view.id).len(), 1);
                    assert_eq!(doc.text().char_to_line(doc.folds(view.id)[0].start), 1);
                }),
            ),
            (
                Some("/hidden<ret>"),
                Some(&|app| {
                    let (view, doc) = current_ref!(app.editor);
                    assert!(doc.folds(view.id).is_empty());
                    assert_eq!(
                        doc.selection(view.id)
                            .primary()
                            .cursor_line(doc.text().slice(..)),
                        2
                    );
                }),
            ),
        ],
        false,
    )
    .await
}
