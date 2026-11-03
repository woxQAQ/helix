use super::*;

use arc_swap::ArcSwap;
use helix_term::{
    compositor::{Compositor, Context, Event},
    job::Jobs,
    keymap::{self, Keymaps},
    ui::EditorView,
};
use helix_view::{current_ref, graphics::Rect, input::parse_macro, view::ViewPosition};
use std::time::Duration;
use tui::buffer::Buffer;

#[tokio::test(flavor = "multi_thread")]
async fn inline_hunk_preserves_wrapping_colors_and_scroll_coordinates() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("wrapped.txt");
    let after = format!("中文\t{}\nsecond\n", "long text ".repeat(8));
    let before = format!("旧文\t{}\nsecond\n", "long text ".repeat(8));
    std::fs::write(&path, &after)?;
    let mut config = helpers::test_config();
    config.editor.soft_wrap.enable = Some(true);
    let mut app = helpers::AppBuilder::new()
        .with_config(config)
        .with_file(&path, None)
        .build()?;
    {
        let (_, doc) = helix_view::current!(app.editor);
        doc.set_diff_base(before.into_bytes());
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if current_ref!(app.editor)
                .1
                .diff_handle()
                .is_some_and(|handle| handle.load().len() == 1)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    let area = Rect::new(3, 2, 28, 20);
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
    let inner = view.inner_area(doc);
    let second = doc.text().line_to_char(1);
    let before_row = view
        .screen_coords_at_pos(doc, doc.text().slice(..), second)
        .unwrap()
        .row;
    assert!(before_row > 1);
    let original_first_cell = surface[(inner.x, inner.y)].clone();
    for key in parse_macro("<space>vh")? {
        compositor.handle_event(&Event::Key(key), &mut cx);
    }
    surface.reset();
    compositor.render(area, &mut surface, &mut cx);
    assert_eq!(surface[(inner.x, inner.y)], original_first_cell);
    let (view, doc) = current_ref!(cx.editor);
    let after_row = view
        .screen_coords_at_pos(doc, doc.text().slice(..), second)
        .unwrap()
        .row;
    assert_eq!(after_row, before_row + 3);
    assert_eq!(
        view.pos_at_screen_coords(doc, inner.y + after_row as u16, inner.x, false),
        Some(second)
    );
    let removed_row = inner.y + before_row as u16 + 1;
    let added_row = removed_row + 1;
    assert_eq!(surface[(inner.x, removed_row)].symbol.as_str(), "-");
    assert_eq!(surface[(inner.x, added_row)].symbol.as_str(), "+");
    assert_eq!(
        Some(surface[(inner.x, removed_row)].fg),
        cx.editor.theme.get("diff.minus").fg
    );
    assert_eq!(
        Some(surface[(inner.x, added_row)].fg),
        cx.editor.theme.get("diff.plus").fg
    );
    // Scroll into the virtual block: its first visible row must still be rendered.
    {
        let (view, doc) = helix_view::current!(cx.editor);
        doc.set_view_offset(
            view.id,
            ViewPosition {
                anchor: 0,
                vertical_offset: before_row + 1,
                horizontal_offset: 0,
            },
        );
    }
    surface.reset();
    compositor.render(area, &mut surface, &mut cx);
    assert_eq!(surface[(inner.x, inner.y)].symbol.as_str(), "-");
    assert_eq!(surface[(inner.x, inner.y + 1)].symbol.as_str(), "+");
    let (_, doc) = current_ref!(cx.editor);
    assert_eq!(doc.text().to_string(), after);
    assert!(!doc.is_modified());
    for key in parse_macro("10]g10[g")? {
        compositor.handle_event(&Event::Key(key), &mut cx);
    }
    let (view, doc) = current_ref!(cx.editor);
    assert_eq!(
        doc.selection(view.id)
            .primary()
            .cursor_line(doc.text().slice(..)),
        0
    );
    // A diff-base refresh invalidates the preview before the asynchronous worker catches up.
    {
        let (view, doc) = helix_view::current!(cx.editor);
        doc.set_view_offset(view.id, ViewPosition::default());
        doc.set_diff_base(after.as_bytes().to_vec());
    }
    surface.reset();
    compositor.render(area, &mut surface, &mut cx);
    let (view, doc) = current_ref!(cx.editor);
    assert_eq!(
        view.screen_coords_at_pos(doc, doc.text().slice(..), second)
            .unwrap()
            .row,
        before_row
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if current_ref!(cx.editor)
                .1
                .diff_handle()
                .is_some_and(|handle| handle.load().is_empty())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    let (view, doc) = current_ref!(cx.editor);
    let selection = doc.selection(view.id).clone();
    for key in parse_macro("]g[g10]g10[g")? {
        compositor.handle_event(&Event::Key(key), &mut cx);
    }
    let (view, doc) = current_ref!(cx.editor);
    assert_eq!(doc.selection(view.id), &selection);
    Ok(())
}
