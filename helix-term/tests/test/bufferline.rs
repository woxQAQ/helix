use super::*;

use arc_swap::ArcSwap;
use helix_core::unicode::width::UnicodeWidthStr;
use helix_term::{
    compositor::{Compositor, Context, Event},
    job::Jobs,
    keymap::{self, Keymaps},
    ui::EditorView,
};
use helix_view::{
    current_ref,
    editor::{Action, BufferLine},
    graphics::Rect,
    input::{parse_macro, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
};
use tui::buffer::Buffer;

#[tokio::test(flavor = "multi_thread")]
async fn bufferline_clicks_follow_rendered_tabs() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let first = directory.path().join("中文.txt");
    let second = directory.path().join("e\u{301}.txt");
    let third = directory.path().join("long-buffer-name.txt");
    for path in [&first, &second, &third] {
        std::fs::write(path, "hello\n")?;
    }

    for sidebar in [false, true] {
        let mut config = helpers::test_config();
        config.editor.bufferline = BufferLine::Always;
        let mut app = helpers::AppBuilder::new()
            .with_config(config)
            .with_file(&first, None)
            .build()?;
        let first_id = current_ref!(app.editor).1.id();
        let second_id = app.editor.open(&second, Action::Load)?;
        let third_id = app.editor.open(&third, Action::Load)?;
        let area = Rect::new(3, 2, 100, 20);
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
        // Keep another split visible and include the modification marker in the first tab.
        for key in parse_macro("iX<esc>:vsplit<ret>")? {
            compositor.handle_event(&Event::Key(key), &mut cx);
            compositor.render(area, &mut surface, &mut cx);
        }
        if sidebar {
            for key in parse_macro("<space>e")? {
                compositor.handle_event(&Event::Key(key), &mut cx);
                compositor.render(area, &mut surface, &mut cx);
            }
        }
        let focused_view = current_ref!(cx.editor).0.id;
        let tab_start = area.x + if sidebar { 30 } else { 0 };
        let second_start = tab_start + " 中文.txt[+] ".width() as u16;
        let third_start = second_start + " e\u{301}.txt ".width() as u16;
        assert_eq!(
            surface
                .get(second_start + 1, area.y)
                .unwrap()
                .symbol
                .as_str(),
            "e\u{301}"
        );

        let click = |compositor: &mut Compositor, cx: &mut Context, column, row| {
            for kind in [
                MouseEventKind::Down(MouseButton::Left),
                MouseEventKind::Up(MouseButton::Left),
            ] {
                compositor.handle_event(
                    &Event::Mouse(MouseEvent {
                        kind,
                        column,
                        row,
                        modifiers: KeyModifiers::empty(),
                    }),
                    cx,
                );
            }
        };
        // The leading padding belongs to the following tab, including after wide filenames.
        click(&mut compositor, &mut cx, second_start, area.y);
        assert_eq!(current_ref!(cx.editor).1.id(), second_id);
        assert_eq!(current_ref!(cx.editor).0.id, focused_view);
        assert!(cx
            .editor
            .tree
            .views()
            .any(|(view, focused)| !focused && view.doc == first_id));
        compositor.render(area, &mut surface, &mut cx);
        click(&mut compositor, &mut cx, third_start + 2, area.y);
        assert_eq!(current_ref!(cx.editor).1.id(), third_id);
        compositor.render(area, &mut surface, &mut cx);
        click(&mut compositor, &mut cx, area.right() - 1, area.y);
        assert_eq!(current_ref!(cx.editor).1.id(), third_id);
        click(&mut compositor, &mut cx, second_start - 1, area.y);
        assert_eq!(current_ref!(cx.editor).1.id(), first_id);
        assert_eq!(current_ref!(cx.editor).1.text().to_string(), "Xhello\n");

        // Resizing hides the sidebar and clips the second tab to its visible cells.
        let narrow = Rect::new(3, 2, 16, 10);
        let mut narrow_surface = Buffer::empty(narrow);
        compositor.render(narrow, &mut narrow_surface, &mut cx);
        click(&mut compositor, &mut cx, narrow.right() - 1, narrow.y);
        assert_eq!(current_ref!(cx.editor).1.id(), second_id);

        // Hiding the bufferline clears its hit regions.
        compositor.render(area, &mut surface, &mut cx);
        click(&mut compositor, &mut cx, tab_start + 1, area.y);
        assert_eq!(current_ref!(cx.editor).1.id(), first_id);
        let mut hidden_config = cx.editor.config().clone();
        hidden_config.bufferline = BufferLine::Never;
        cx.editor.config = std::sync::Arc::new(ArcSwap::from_pointee(hidden_config));
        compositor.render(area, &mut surface, &mut cx);
        click(&mut compositor, &mut cx, second_start + 1, area.y);
        assert_eq!(current_ref!(cx.editor).1.id(), first_id);
        helpers::assert_status_not_error(cx.editor);
    }
    Ok(())
}
