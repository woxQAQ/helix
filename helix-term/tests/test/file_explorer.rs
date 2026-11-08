use super::*;

use helix_view::{current_ref, graphics::Rect};

#[tokio::test(flavor = "multi_thread")]
async fn sidebar_navigation_preserves_layout_and_document_focus() -> anyhow::Result<()> {
    let workspace = helix_loader::find_workspace().0;
    let directory = tempfile::Builder::new()
        .prefix("sidebar-test-")
        .tempdir_in(workspace)?;
    let first = directory.path().join("first.txt");
    let nested = directory.path().join("nested");
    let child = nested.join("child.txt");
    std::fs::create_dir(&nested)?;
    std::fs::write(&first, "first\n")?;
    std::fs::write(&child, "child\n")?;
    std::fs::write(nested.join(".hidden.txt"), "hidden\n")?;
    let added = nested.join("next.txt");

    let mut config = helpers::test_config();
    config.editor.file_explorer.flatten_dirs = false;
    config.editor.file_explorer.hidden = true;
    config.editor.file_explorer.sidebar_width = 32;
    let mut app = helpers::AppBuilder::new()
        .with_config(config)
        .with_file(&first, None)
        .build()?;

    test_key_sequences(
        &mut app,
        vec![
            (
                Some("<space>eixdd<esc>"),
                Some(&|app| {
                    let (view, doc) = current_ref!(app.editor);
                    assert_eq!(view.area.x, 32);
                    assert_eq!(app.editor.tree.views().count(), 1);
                    assert_eq!(doc.text().to_string(), "first\n");
                    assert!(!doc.is_modified());
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (
                Some("hllj<ret>"),
                Some(&|app| {
                    let (view, doc) = current_ref!(app.editor);
                    assert_eq!(doc.path(), Some(child.as_path()));
                    assert_eq!(view.area.x, 32);
                    assert_eq!(app.editor.tree.views().count(), 1);
                    helpers::assert_status_not_error(&app.editor);
                    std::fs::write(&added, "next\n").unwrap();
                }),
            ),
            (
                Some("<space>ERj<ret>"),
                Some(&|app| {
                    let (_, doc) = current_ref!(app.editor);
                    assert_eq!(doc.path(), Some(added.as_path()));
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (
                Some("iX<esc><space>E<C-v>"),
                Some(&|app| {
                    let (_, doc) = current_ref!(app.editor);
                    assert_eq!(doc.text().to_string(), "Xnext\n");
                    assert_eq!(app.editor.tree.views().count(), 2);
                    assert!(app.editor.tree.views().all(|(view, _)| view.area.x >= 32));
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (
                Some("<space>e"),
                Some(&|app| {
                    assert_eq!(
                        app.editor.tree.views().map(|(view, _)| view.area.x).min(),
                        Some(0)
                    );
                    assert_eq!(app.editor.tree.views().count(), 2);
                }),
            ),
            (Some(":qa!<ret>"), None),
        ],
        true,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn sidebar_create_cancels_rejects_conflicts_and_selects_file() -> anyhow::Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("sidebar-create-test-")
        .tempdir_in(helix_loader::find_workspace().0)?;
    let first = directory.path().join("first.txt");
    let created = directory.path().join("added.txt");
    std::fs::write(&first, "first\n")?;
    let mut app = helpers::AppBuilder::new().with_file(&first, None).build()?;

    test_key_sequences(
        &mut app,
        vec![
            (
                Some("<space>eacancelled.txt<esc>a<ret>"),
                Some(&|app| {
                    assert!(!directory.path().join("cancelled.txt").exists());
                    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (
                Some("afirst.txt<ret>"),
                Some(&|app| {
                    assert_eq!(std::fs::read_to_string(&first).unwrap(), "first\n");
                    assert!(app.editor.status_msg.as_ref().unwrap().0.contains("exist"));
                }),
            ),
            (
                Some("a../outside.txt<ret>"),
                Some(&|app| {
                    assert!(app
                        .editor
                        .status_msg
                        .as_ref()
                        .unwrap()
                        .0
                        .contains("directory components"));
                }),
            ),
            (
                Some("aadded.txt<ret>"),
                Some(&|app| {
                    assert_eq!(std::fs::read(&created).unwrap(), b"");
                    assert_eq!(current_ref!(app.editor).1.path(), Some(first.as_path()));
                    assert!(!current_ref!(app.editor).1.is_modified());
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (
                Some("<ret>"),
                Some(&|app| {
                    assert_eq!(current_ref!(app.editor).1.path(), Some(created.as_path()));
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (Some(":qa!<ret>"), None),
        ],
        true,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn sidebar_create_directory_selects_it_and_creates_child() -> anyhow::Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("sidebar-create-directory-test-")
        .tempdir_in(helix_loader::find_workspace().0)?;
    let first = directory.path().join("first.txt");
    let created = directory.path().join("a");
    let child = created.join("child.txt");
    std::fs::write(&first, "first\n")?;
    let mut app = helpers::AppBuilder::new().with_file(&first, None).build()?;

    test_key_sequences(
        &mut app,
        vec![
            (
                Some("<space>eafirst.txt/<ret>"),
                Some(&|app| {
                    assert_eq!(std::fs::read_to_string(&first).unwrap(), "first\n");
                    assert!(app.editor.status_msg.as_ref().unwrap().0.contains("exist"));
                }),
            ),
            (
                Some("aa/<ret>"),
                Some(&|app| {
                    assert!(created.is_dir());
                    assert_eq!(current_ref!(app.editor).1.path(), Some(first.as_path()));
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (
                Some("jaa/<ret>"),
                Some(&|app| {
                    assert!(!created.join("a").exists());
                    assert!(app.editor.status_msg.as_ref().unwrap().0.contains("exist"));
                }),
            ),
            (
                Some("kachild.txt<ret>"),
                Some(&|app| {
                    assert_eq!(std::fs::read(&child).unwrap(), b"");
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (
                Some("<ret>"),
                Some(&|app| {
                    assert_eq!(current_ref!(app.editor).1.path(), Some(child.as_path()));
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (Some(":qa!<ret>"), None),
        ],
        true,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn sidebar_create_in_collapsed_directory_reveals_file() -> anyhow::Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("sidebar-create-dir-test-")
        .tempdir_in(helix_loader::find_workspace().0)?;
    let nested = directory.path().join("nested");
    std::fs::create_dir(&nested)?;
    let first = directory.path().join("first.txt");
    let created = nested.join("child.txt");
    std::fs::write(&first, "first\n")?;
    let mut config = helpers::test_config();
    config.editor.file_explorer.flatten_dirs = false;
    let mut app = helpers::AppBuilder::new()
        .with_config(config)
        .with_file(&first, None)
        .build()?;

    test_key_sequences(
        &mut app,
        vec![
            (
                Some("<space>ekachild.txt<ret>"),
                Some(&|app| {
                    assert_eq!(std::fs::read(&created).unwrap(), b"");
                    assert!(!directory.path().join("child.txt").exists());
                    assert_eq!(current_ref!(app.editor).1.path(), Some(first.as_path()));
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (
                Some("<ret>"),
                Some(&|app| {
                    assert_eq!(current_ref!(app.editor).1.path(), Some(created.as_path()));
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (Some(":qa!<ret>"), None),
        ],
        true,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn sidebar_delete_requires_confirmation_and_refreshes() -> anyhow::Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("sidebar-delete-test-")
        .tempdir_in(helix_loader::find_workspace().0)?;
    let first = directory.path().join("first.txt");
    let second = directory.path().join("second.txt");
    std::fs::write(&first, "first\n")?;
    std::fs::write(&second, "second\n")?;
    let mut app = helpers::AppBuilder::new().with_file(&first, None).build()?;

    test_key_sequences(
        &mut app,
        vec![
            (
                Some("<space>ed"),
                Some(&|app| {
                    assert!(first.exists());
                    assert_eq!(current_ref!(app.editor).1.path(), Some(first.as_path()));
                }),
            ),
            (
                Some("<esc>d<ret>dn<ret>dy<esc>"),
                Some(&|app| {
                    assert!(first.exists());
                    assert!(!current_ref!(app.editor).1.is_modified());
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (
                Some("dy<ret>"),
                Some(&|app| {
                    assert!(!first.exists());
                    assert!(second.exists());
                    assert_eq!(current_ref!(app.editor).1.text().to_string(), "first\n");
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (
                Some("<ret>"),
                Some(&|app| {
                    assert_eq!(current_ref!(app.editor).1.path(), Some(second.as_path()));
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (
                Some("<space>Edyes<ret>"),
                Some(&|app| {
                    assert!(!second.exists());
                    assert_eq!(current_ref!(app.editor).1.text().to_string(), "second\n");
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (Some(":qa!<ret>"), None),
        ],
        true,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn sidebar_directory_delete_requires_confirmation_and_refreshes() -> anyhow::Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("sidebar-delete-dir-test-")
        .tempdir_in(helix_loader::find_workspace().0)?;
    let empty = directory.path().join("empty");
    let nested = directory.path().join("nested");
    let child = nested.join("deeper/child.txt");
    let remaining = directory.path().join("remaining.txt");
    std::fs::create_dir(&empty)?;
    std::fs::create_dir_all(child.parent().unwrap())?;
    std::fs::write(&child, "child\n")?;
    std::fs::write(&remaining, "remaining\n")?;
    let mut config = helpers::test_config();
    config.editor.file_explorer.flatten_dirs = false;
    let mut app = helpers::AppBuilder::new()
        .with_config(config)
        .with_file(&child, None)
        .build()?;

    test_key_sequences(
        &mut app,
        vec![
            (
                Some("<space>ehhhhkd"),
                Some(&|app| {
                    assert!(empty.is_dir());
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (
                Some("<esc>d<ret>dn<ret>dy<esc>"),
                Some(&|app| {
                    assert!(empty.is_dir());
                    assert!(child.exists());
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (
                Some("dy<ret>"),
                Some(&|app| {
                    assert!(!empty.exists());
                    assert!(child.exists());
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (
                Some("dyes<ret>"),
                Some(&|app| {
                    assert!(!nested.exists());
                    assert!(remaining.exists());
                    assert_eq!(current_ref!(app.editor).1.text().to_string(), "child\n");
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (
                Some("<ret>"),
                Some(&|app| {
                    assert_eq!(current_ref!(app.editor).1.path(), Some(remaining.as_path()));
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (Some(":qa!<ret>"), None),
        ],
        true,
    )
    .await
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn sidebar_delete_directory_symlink_preserves_target() -> anyhow::Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("sidebar-delete-symlink-test-")
        .tempdir_in(helix_loader::find_workspace().0)?;
    let target = directory.path().join("target");
    let link = directory.path().join("link");
    let child = target.join("child.txt");
    std::fs::create_dir(&target)?;
    std::fs::write(&child, "child\n")?;
    std::os::unix::fs::symlink(&target, &link)?;
    let mut config = helpers::test_config();
    config.editor.file_explorer.flatten_dirs = false;
    config.editor.file_explorer.follow_symlinks = true;
    let mut app = helpers::AppBuilder::new()
        .with_config(config)
        .with_file(&child, None)
        .build()?;

    test_key_sequences(
        &mut app,
        vec![
            (
                Some("<space>ehkdy<ret>"),
                Some(&|app| {
                    assert!(std::fs::symlink_metadata(&link).is_err());
                    assert!(target.is_dir());
                    assert_eq!(std::fs::read_to_string(&child).unwrap(), "child\n");
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (Some(":qa!<ret>"), None),
        ],
        true,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn sidebar_rename_preserves_buffer_and_rejects_conflicts() -> anyhow::Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("sidebar-rename-test-")
        .tempdir_in(helix_loader::find_workspace().0)?;
    let first = directory.path().join("first.txt");
    let second = directory.path().join("second.txt");
    let renamed = directory.path().join("z-renamed.txt");
    std::fs::write(&first, "first\n")?;
    std::fs::write(&second, "second\n")?;
    let mut app = helpers::AppBuilder::new().with_file(&first, None).build()?;
    let doc_id = current_ref!(app.editor).1.id();

    test_key_sequences(
        &mut app,
        vec![
            (
                Some("iX<esc><space>er<C-u>cancelled.txt<esc>r<ret>r<C-u><ret>"),
                Some(&|app| {
                    assert!(first.exists());
                    assert!(!directory.path().join("cancelled.txt").exists());
                    assert_eq!(current_ref!(app.editor).1.path(), Some(first.as_path()));
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (
                Some("r<C-u>second.txt<ret>"),
                Some(&|app| {
                    assert!(first.exists());
                    assert_eq!(std::fs::read_to_string(&second).unwrap(), "second\n");
                    assert!(app
                        .editor
                        .status_msg
                        .as_ref()
                        .unwrap()
                        .0
                        .contains("already exists"));
                }),
            ),
            (
                Some("r<C-u>../outside.txt<ret>"),
                Some(&|app| {
                    assert!(first.exists());
                    assert!(app
                        .editor
                        .status_msg
                        .as_ref()
                        .unwrap()
                        .0
                        .contains("directory components"));
                }),
            ),
            (
                Some("r<C-u>z-renamed.txt<ret><ret>"),
                Some(&|app| {
                    assert!(!first.exists());
                    assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "first\n");
                    let (_, doc) = current_ref!(app.editor);
                    assert_eq!(doc.id(), doc_id);
                    assert_eq!(doc.path(), Some(renamed.as_path()));
                    assert_eq!(doc.text().to_string(), "Xfirst\n");
                    assert!(doc.is_modified());
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (Some(":qa!<ret>"), None),
        ],
        true,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn sidebar_directory_rename_updates_open_descendants() -> anyhow::Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("sidebar-rename-dir-test-")
        .tempdir_in(helix_loader::find_workspace().0)?;
    let nested = directory.path().join("nested");
    let renamed = directory.path().join("renamed");
    std::fs::create_dir(&nested)?;
    let child = nested.join("child.txt");
    let renamed_child = renamed.join("child.txt");
    std::fs::write(&child, "child\n")?;
    let mut config = helpers::test_config();
    config.editor.file_explorer.flatten_dirs = false;
    let mut app = helpers::AppBuilder::new()
        .with_config(config)
        .with_file(&child, None)
        .build()?;
    let doc_id = current_ref!(app.editor).1.id();

    test_key_sequences(
        &mut app,
        vec![
            (
                Some("<space>ehr<C-u>renamed<ret>l<ret>"),
                Some(&|app| {
                    assert!(!nested.exists());
                    assert!(renamed_child.exists());
                    let (_, doc) = current_ref!(app.editor);
                    assert_eq!(doc.id(), doc_id);
                    assert_eq!(doc.path(), Some(renamed_child.as_path()));
                    assert_eq!(doc.text().to_string(), "child\n");
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (Some(":qa!<ret>"), None),
        ],
        true,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn sidebar_mouse_and_terminal_resize() -> anyhow::Result<()> {
    use arc_swap::ArcSwap;
    use helix_term::{
        compositor::{Compositor, Context, Event},
        job::Jobs,
        keymap::{self, Keymaps},
        ui::EditorView,
    };
    use helix_view::input::{parse_macro, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use tui::buffer::Buffer;

    let directory = tempfile::Builder::new()
        .prefix("sidebar-test-")
        .tempdir_in(helix_loader::find_workspace().0)?;
    let first = directory.path().join("first.txt");
    std::fs::write(&first, "first\n")?;
    let mut config = helpers::test_config();
    config.editor.bufferline = helix_view::editor::BufferLine::Always;
    let mut app = helpers::AppBuilder::new()
        .with_config(config)
        .with_file(&first, None)
        .build()?;
    let area = Rect::new(0, 0, 100, 20);
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

    for key in parse_macro("<space>e")? {
        compositor.handle_event(&Event::Key(key), &mut cx);
        compositor.render(area, &mut surface, &mut cx);
    }
    let selected_row = (1..area.height - 1)
        .find(|&y| {
            let row: String = (0..29)
                .map(|x| surface.get(x, y).unwrap().symbol.as_str())
                .collect();
            row.contains("first.txt")
        })
        .expect("the current file should be revealed in the sidebar");
    assert_eq!(surface.get(29, selected_row).unwrap().symbol.as_str(), "│");
    assert_eq!(compositor.cursor(area, cx.editor).0, None);

    // A mouse click into the tree cancels a pending document command prefix.
    for key in parse_macro("<esc>g")? {
        compositor.handle_event(&Event::Key(key), &mut cx);
    }
    compositor.handle_event(
        &Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 3,
            row: selected_row,
            modifiers: KeyModifiers::empty(),
        }),
        &mut cx,
    );
    for key in parse_macro("iX<esc>")? {
        compositor.handle_event(&Event::Key(key), &mut cx);
    }
    assert_eq!(helix_view::doc!(cx.editor).text().to_string(), "Xfirst\n");

    // A small terminal temporarily hides the tree and returns all space to the document.
    for small in [Rect::new(0, 0, 25, 10), Rect::new(0, 0, 1, 1)] {
        let mut small_surface = Buffer::empty(small);
        compositor.render(small, &mut small_surface, &mut cx);
        assert_eq!(cx.editor.tree.views().next().unwrap().0.area.x, 0);
    }
    compositor.render(area, &mut surface, &mut cx);
    assert_eq!(cx.editor.tree.views().next().unwrap().0.area.x, 30);
    assert_eq!(surface.get(29, selected_row).unwrap().symbol.as_str(), "│");
    Ok(())
}
