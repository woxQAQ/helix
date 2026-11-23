use super::*;
use helix_term::application::Application;
use helix_view::current_ref;

#[tokio::test(flavor = "multi_thread")]
async fn review_comments_render_below_code_and_follow_edits() -> anyhow::Result<()> {
    use arc_swap::ArcSwap;
    use helix_term::{
        compositor::{Compositor, Context, Event},
        job::Jobs,
        keymap::{self, Keymaps},
        ui::EditorView,
    };
    use helix_view::{
        graphics::Rect,
        input::{parse_macro, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    };
    use tui::buffer::Buffer;

    let dir = tempfile::tempdir()?;
    let path = dir.path().join("review.txt");
    let source = "first\r\nsecond\r\nthird";
    std::fs::write(&path, source)?;
    let mut config = test_config();
    config.editor.soft_wrap.enable = Some(true);
    let mut app = AppBuilder::new()
        .with_config(config)
        .with_comments()
        .with_file(&path, None)
        .build()?;
    let area = Rect::new(2, 1, 39, 28);
    let mut compositor = Compositor::new(area);
    let mut keys = keymap::default();
    keymap::merge_keys(
        &mut keys,
        toml::from_str("[normal]\nF4 = 'review_comment'")?,
    );
    compositor.push(Box::new(EditorView::new(Keymaps::new(Box::new(
        ArcSwap::from_pointee(keys),
    )))));
    let mut jobs = Jobs::new();
    let mut cx = Context {
        editor: &mut app.editor,
        scroll: None,
        jobs: &mut jobs,
    };
    let mut surface = Buffer::empty(area);
    let mut frame = |keys: &str| -> anyhow::Result<Vec<String>> {
        for key in parse_macro(keys)? {
            compositor.handle_event(&Event::Key(key), &mut cx);
        }
        surface.reset();
        compositor.render(area, &mut surface, &mut cx);
        let (view, doc) = current_ref!(cx.editor);
        let inner = view.inner_area(doc);
        if let Some((focus, col)) = doc.review_caret(view.id, inner.width) {
            let (pos, shape) = compositor.cursor(area, cx.editor);
            assert_eq!(shape, helix_view::graphics::CursorKind::Bar);
            let pos = pos.expect("inline draft has a visible caret");
            assert_eq!(pos.col, usize::from(inner.x) + col);
            assert_eq!(
                view.review_at_screen_coords(doc, pos.row as u16, pos.col as u16),
                Some(focus)
            );
        }
        // Include the range guide in the snapshot when a multiline comment spans this row.
        let rows = (inner.top()..inner.bottom())
            .map(|y| {
                let row: String = (inner.left() - 1..inner.right())
                    .map(|x| surface[(x, y)].symbol.as_str())
                    .collect();
                row.trim().to_owned()
            })
            .collect();
        Ok(rows)
    };
    let creating = frame("2x<F4>new<A-ret>block")?;
    assert!(creating[2].starts_with("├── L1–L2 "));
    assert!(creating[3].contains("new") && creating[4].contains("block"));
    assert_eq!(&frame("<esc>gg")?[..3], &["first", "second", "third"]);
    let rows = frame(":comment Overview<ret>:comment First note<ret>2x:comment Both lines<ret>:3<ret>:comment 检查这行 ABCDEFGHIJKLMNOPQRSTUVWXYZ<ret>")?;
    assert_eq!(rows[0], "╭first");
    assert!(rows[1].starts_with("│╭─ L1 "));
    assert!(rows[2].starts_with("││ Overview ") && rows[2].ends_with('│'));
    assert!(rows[3].starts_with("│╰─") && rows[3].ends_with('╯'));
    assert!(rows[5].starts_with("││ First note "));
    assert_eq!(rows[7], "│second");
    assert!(rows[8].starts_with("├── L1–L2 ") && rows[8].ends_with('╮'));
    assert!(rows[9].starts_with("│  Both lines ") && rows[9].ends_with('│'));
    assert_eq!(rows[11], "third");
    assert!(rows[13].starts_with("│ 检"));
    assert!(rows[14].contains("XYZ"));
    let moved = frame("ggOheader<esc>")?;
    assert_eq!(&moved[..2], &["header", "╭first"]);
    assert!(moved[2].starts_with("│╭─ L2 "));
    assert!(moved[9].starts_with("├── L2–L3 "));
    let removed = frame(":2<ret>2xd")?;
    assert!(removed.iter().any(|line| line.contains("(target removed)")));
    assert_eq!(frame("uu")?, rows);
    let wrapped = frame(&format!("ggA{} tail<esc>", " woord".repeat(8)))?;
    let last_segment = wrapped
        .iter()
        .position(|row| row.ends_with("tail"))
        .unwrap();
    assert!(last_segment > 0);
    assert!(wrapped[last_segment + 2].contains("│ Overview "));
    assert!(wrapped[last_segment + 5].contains("│ First note "));
    assert_eq!(frame("u")?, rows);
    let draft = frame("ggjj<ret> updated")?;
    assert!(draft[2].contains("│ Overview updated "));
    let edited = frame("<ret>")?;
    assert_eq!(draft, edited);
    let live = frame("i<A-ret>第二行<up>!<down>?")?;
    assert!(live[2].contains("Overvi!ew updated"), "{live:?}");
    assert!(live[3].contains('第'), "{live:?}");
    let canceled = frame("<esc>")?;
    assert_eq!(canceled, edited);
    let second_note = frame("jjj<ret> amended<ret>")?;
    assert!(second_note[5].contains("│ First note amended "));
    frame("jjk")?;
    let (view, doc) = current_ref!(cx.editor);
    let focus = doc.review_focus(view.id).unwrap();
    assert_eq!(doc.review_text(focus.id), Some("First note amended"));
    let screen = cx.editor.cursor().0.unwrap();
    assert_eq!(
        view.review_at_screen_coords(doc, screen.row as u16, screen.col as u16),
        Some(focus)
    );
    assert_eq!(doc.text().to_string(), source);
    assert!(!doc.is_modified());
    let inner = view.inner_area(doc);
    assert_eq!(
        view.pos_at_screen_coords(doc, inner.y + 7, inner.x, false),
        Some(doc.text().line_to_char(1))
    );
    // Focus the EOF comment by clicking its body, then traverse a comment taller than the split.
    compositor.handle_event(
        &Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            row: inner.y + 13,
            column: inner.x + 3,
            modifiers: KeyModifiers::empty(),
        }),
        &mut cx,
    );
    let long_text = "wide text ".repeat(100);
    let keys = format!("<ret><C-u>{long_text}end<ret>{}", "j".repeat(32));
    for key in parse_macro(&keys)? {
        compositor.handle_event(&Event::Key(key), &mut cx);
    }
    surface.reset();
    compositor.render(area, &mut surface, &mut cx);
    let (view, doc) = current_ref!(cx.editor);
    let focus = doc.review_focus(view.id).unwrap();
    assert!(focus.row >= 30);
    let screen = cx.editor.cursor().0.unwrap();
    assert_eq!(
        view.review_at_screen_coords(doc, screen.row as u16, screen.col as u16),
        Some(focus)
    );
    assert_eq!(doc.text().to_string(), source);
    let report = cx.editor.review.as_ref().unwrap().report(cx.editor);
    assert!(report.contains(&format!("review.txt:3 — {long_text}end\n")));
    assert!(report.contains("review.txt:1 — First note amended\n"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn review_across_edits_history_and_closed_buffers() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let base = helix_stdx::path::canonicalize(dir.path());
    let path = base.join("alpha.txt");
    let other = base.join("beta.txt");
    std::fs::write(&path, "first\r\n中文\r\nthird\r\nlast\r\n")?;
    std::fs::write(&other, "other\n")?;
    let mut app = AppBuilder::new()
        .with_comments()
        .with_file(&path, None)
        .build()?;
    let open_other = format!(
        ":open '{}'<ret>:comment Other file<ret>",
        other.display().to_string().replace('\'', "''")
    );
    let report = |app: &Application| app.editor.review.as_ref().unwrap().report(&app.editor);
    test_key_sequences(&mut app, vec![
        (Some("<space>ncDiscard this<esc><space>ncOverview<ret>:2<ret><space>nc100% '%{buffer_name}'<ret>v2x<space>ncTwo lines<ret><esc>"), Some(&|app| {
            let text = report(app);
            assert!(!text.contains("Discard this"));
            assert!(text.contains("alpha.txt:1 — Overview\n"), "{text}");
            assert!(text.contains("alpha.txt:2 — 100% '%{buffer_name}'\n"), "{text}");
            assert!(text.contains("alpha.txt:2-3 — Two lines\n"), "{text}");
        })),
        (Some("d"), Some(&|app| {
            assert!(report(app).contains("alpha.txt:2 (target removed) — Two lines"));
        })),
        (Some("u"), Some(&|app| {
            assert!(report(app).contains("alpha.txt:2-3 — Two lines"));
        })),
        (Some("U"), Some(&|app| {
            assert!(report(app).contains("alpha.txt:2 (target removed) — Two lines"));
        })),
        (Some("uggOheader<esc>"), Some(&|app| {
            assert!(report(app).contains("alpha.txt:3-4 — Two lines"));
        })),
        (Some(":w<ret>:bc<ret>"), Some(&|app| {
            assert!(app.editor.document_id_by_path(&path).is_none());
            assert!(report(app).contains("alpha.txt:3-4 — Two lines"));
            std::fs::write(&path, "outside\r\nheader\r\nfirst\r\n中文\r\nthird\r\nlast\r\n").unwrap();
        })),
        (Some(&open_other), Some(&|app| {
            assert!(report(app).contains("beta.txt:1 — Other file"));
        })),
        (Some("<space>np"), None),
        (Some("Overview"), None),
        (Some("<del>"), Some(&|app| {
            assert!(!report(app).contains("Overview"));
            assert!(app.editor.document_id_by_path(&path).is_none());
        })),
        (Some("<esc><space>np"), None),
        (Some("Two lines<ret>"), Some(&|app| {
            let (view, doc) = current_ref!(app.editor);
            assert_eq!(doc.path(), Some(path.as_path()));
            assert_eq!(doc.selection(view.id).primary().fragment(doc.text().slice(..)), "中文\r\nthird\r\n");
            assert!(report(app).contains("alpha.txt:4-5 — Two lines"));
        })),
        (Some(":4<ret>l<space>ncDuplicate target<ret><space>np"), None),
        (Some("Duplicate target"), None),
        (Some("<del><esc>"), Some(&|app| {
            let text = report(app);
            assert!(!text.contains("Duplicate target"));
            assert!(text.contains("alpha.txt:4 — 100% '%{buffer_name}'"));
            assert!(text.contains("alpha.txt:4-5 — Two lines"));
            let (_, doc) = current_ref!(app.editor);
            let anchor = helix_core::line_ending::line_end_char_index(&doc.text().slice(..), 3);
            let notes = doc.review_annotations(80).unwrap();
            assert_eq!(notes.rows_at(anchor).map(|row| row.text).collect::<Vec<_>>(), ["L4", "100% '%{buffer_name}'", ""]);
        })),
    ], false).await
}
