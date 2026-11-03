use super::*;

use arc_swap::ArcSwap;
use helix_term::{
    compositor::{Compositor, Context, Event},
    job::Jobs,
    keymap::{self, Keymaps},
    ui::EditorView,
};
use helix_view::{current_ref, graphics::Rect, input::parse_macro};
use std::{process::Command, time::Duration};
use tui::buffer::Buffer;

#[tokio::test(flavor = "multi_thread")]
async fn git_blame_and_hunk_workflow() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let git = |args: &[&str]| -> anyhow::Result<()> {
        let output = Command::new("git")
            .arg("-C")
            .arg(directory.path())
            .args(args)
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(())
    };
    git(&["init", "-q"])?;
    let nested = directory.path().join("中文 space");
    std::fs::create_dir(&nested)?;
    let path = nested.join("--file.txt");
    let base = "first\nsecond\nthird\nfourth\nfifth\n";
    std::fs::write(&path, base)?;
    git(&["add", "."])?;
    git(&[
        "-c",
        "user.name=Test Author",
        "-c",
        "user.email=test@example.com",
        "commit",
        "-qm",
        "initial subject",
    ])?;

    // Register this queue before constructing the application, whose editor is used below.
    let mut jobs = Jobs::new();
    let mut app = helpers::AppBuilder::new().with_file(&path, None).build()?;
    let area = Rect::new(3, 2, 120, 20);
    let mut compositor = Compositor::new(area);
    compositor.push(Box::new(EditorView::new(Keymaps::new(Box::new(
        ArcSwap::from_pointee(keymap::default()),
    )))));
    let mut cx = Context {
        editor: &mut app.editor,
        scroll: None,
        jobs: &mut jobs,
    };
    let mut surface = Buffer::empty(area);
    let keys =
        |sequence: &str, compositor: &mut Compositor, cx: &mut Context| -> anyhow::Result<()> {
            for key in parse_macro(sequence)? {
                compositor.handle_event(&Event::Key(key), cx);
            }
            Ok(())
        };
    let screen = |surface: &Buffer| -> String {
        surface
            .content
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect()
    };

    keys("<space>vb", &mut compositor, &mut cx)?;
    let callback = tokio::time::timeout(Duration::from_secs(10), cx.jobs.callbacks.recv())
        .await?
        .unwrap();
    cx.jobs
        .handle_callback(cx.editor, &mut compositor, Ok(Some(callback)));
    compositor.render(area, &mut surface, &mut cx);
    assert!(screen(&surface).contains("Test Author · initial subject"));
    assert_eq!(current_ref!(cx.editor).1.text().to_string(), base);

    // Unsaved edits hide old blame immediately, then resolve against the buffer snapshot.
    keys("iX<esc>", &mut compositor, &mut cx)?;
    surface.reset();
    compositor.render(area, &mut surface, &mut cx);
    assert!(!screen(&surface).contains("initial subject"));
    compositor.handle_event(&Event::IdleTimeout, &mut cx);
    let callback = tokio::time::timeout(Duration::from_secs(10), cx.jobs.callbacks.recv())
        .await?
        .unwrap();
    cx.jobs
        .handle_callback(cx.editor, &mut compositor, Ok(Some(callback)));
    compositor.render(area, &mut surface, &mut cx);
    assert!(screen(&surface).contains("Not committed yet"));

    // Wait for the real diff worker, then preview and navigate two separated hunks.
    keys("4jghiY<esc>gg", &mut compositor, &mut cx)?;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let ready = current_ref!(cx.editor)
                .1
                .diff_handle()
                .is_some_and(|handle| handle.load().len() == 2);
            if ready {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    keys("<space>vh", &mut compositor, &mut cx)?;
    compositor.render(area, &mut surface, &mut cx);
    let rendered = screen(&surface);
    assert!(rendered.contains("-first"));
    assert!(rendered.contains("+Xfirst"));
    assert!(!rendered.contains("No newline at end of file"));
    let (view, doc) = current_ref!(cx.editor);
    let next_line = doc.text().line_to_char(1);
    assert_eq!(
        view.screen_coords_at_pos(doc, doc.text().slice(..), next_line)
            .unwrap()
            .row,
        4
    );
    let inner = view.inner_area(doc);
    assert_eq!(
        view.pos_at_screen_coords(doc, inner.y + 4, inner.x, false),
        Some(next_line)
    );
    assert_eq!(
        view.pos_at_screen_coords(doc, inner.y + 2, inner.x, true),
        None
    );
    // The preview does not capture input; toggle it closed and open in the same view.
    keys("<space>vh", &mut compositor, &mut cx)?;
    surface.reset();
    compositor.render(area, &mut surface, &mut cx);
    assert!(!screen(&surface).contains("-first"));
    let (view, doc) = current_ref!(cx.editor);
    assert_eq!(
        view.screen_coords_at_pos(doc, doc.text().slice(..), next_line)
            .unwrap()
            .row,
        1
    );
    keys("<space>vh<space>vj", &mut compositor, &mut cx)?;
    let (view, doc) = current_ref!(cx.editor);
    assert_eq!(
        doc.selection(view.id)
            .primary()
            .cursor_line(doc.text().slice(..)),
        4
    );
    keys("<space>vk", &mut compositor, &mut cx)?;
    let (view, doc) = current_ref!(cx.editor);
    assert_eq!(
        doc.selection(view.id)
            .primary()
            .cursor_line(doc.text().slice(..)),
        0
    );
    // Both directions and counted motions wrap through the same ring of hunks.
    for (sequence, expected_line) in [
        ("[g", 4),
        ("]g", 0),
        ("5]g", 4),
        ("4]g", 4),
        ("5[g", 0),
        ("4[g", 0),
        ("<space>vk", 4),
        ("<space>vj", 0),
    ] {
        keys(sequence, &mut compositor, &mut cx)?;
        let (view, doc) = current_ref!(cx.editor);
        assert_eq!(
            doc.selection(view.id)
                .primary()
                .cursor_line(doc.text().slice(..)),
            expected_line,
            "{sequence}"
        );
    }
    keys("<space>vb", &mut compositor, &mut cx)?;
    surface.reset();
    compositor.render(area, &mut surface, &mut cx);
    assert!(!screen(&surface).contains("Not committed yet"));
    assert_eq!(
        current_ref!(cx.editor).1.text().to_string(),
        "Xfirst\nsecond\nthird\nfourth\nYfifth\n"
    );
    // A result arriving after blame is disabled must not re-enable its display.
    keys("<space>vb<space>vb", &mut compositor, &mut cx)?;
    let callback = tokio::time::timeout(Duration::from_secs(10), cx.jobs.callbacks.recv())
        .await?
        .unwrap();
    cx.jobs
        .handle_callback(cx.editor, &mut compositor, Ok(Some(callback)));
    surface.reset();
    compositor.render(area, &mut surface, &mut cx);
    assert!(!screen(&surface).contains("Not committed yet"));

    // A pure deletion has no added lines but remains previewable at its gutter position.
    keys("ggxd", &mut compositor, &mut cx)?;
    surface.reset();
    compositor.render(area, &mut surface, &mut cx);
    assert!(!screen(&surface).contains("-first"));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let (_, doc) = current_ref!(cx.editor);
            if doc.diff_handle().is_some_and(|handle| {
                let diff = handle.load();
                diff.doc() == doc.text() && diff.nth_hunk(0).is_pure_removal()
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    keys("<space>vh", &mut compositor, &mut cx)?;
    surface.reset();
    compositor.render(area, &mut surface, &mut cx);
    let rendered = screen(&surface);
    assert!(rendered.contains("-first"));
    assert!(rendered.contains("@@ -1,1 +0,0 @@"));
    assert!(!rendered.contains("+Xfirst"));
    assert!(!rendered.contains("No newline at end of file"));

    // Switching to a scratch buffer invalidates an in-flight file request.
    keys(
        "<space>vb:new<ret>iScratch content<esc>",
        &mut compositor,
        &mut cx,
    )?;
    compositor.handle_event(&Event::IdleTimeout, &mut cx);
    let callback = tokio::time::timeout(Duration::from_secs(10), cx.jobs.callbacks.recv())
        .await?
        .unwrap();
    cx.jobs
        .handle_callback(cx.editor, &mut compositor, Ok(Some(callback)));
    surface.reset();
    compositor.render(area, &mut surface, &mut cx);
    assert!(current_ref!(cx.editor).1.path().is_none());
    assert!(!screen(&surface).contains("initial subject"));
    assert!(!screen(&surface).contains("Not committed yet"));
    Ok(())
}
