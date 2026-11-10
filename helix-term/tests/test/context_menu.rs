use super::*;

use arc_swap::ArcSwap;
use helix_core::unicode::width::UnicodeWidthStr;
use helix_term::{
    application::Application,
    compositor::{Compositor, Context, Event},
    job::Jobs,
    keymap::{self, Keymaps},
    ui::{terminal::Terminal, EditorView},
};
use helix_view::{
    current_ref,
    editor::{Action, BufferLine},
    graphics::{CursorKind, Rect},
    input::{parse_macro, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
};
use std::{path::Path, time::Duration};
use tui::buffer::Buffer;

struct Harness {
    app: Application,
    compositor: Compositor,
    jobs: Jobs,
    area: Rect,
    surface: Buffer,
}

impl Harness {
    fn new(path: &Path) -> anyhow::Result<Self> {
        // Own the runtime's dispatch queue so deferred file operations use this compositor.
        let jobs = Jobs::new();
        let mut config = helpers::test_config();
        config.editor.bufferline = BufferLine::Always;
        config.editor.clipboard_provider = helix_view::clipboard::ClipboardProvider::None;
        config.editor.file_explorer.flatten_dirs = false;
        let app = helpers::AppBuilder::new()
            .with_config(config)
            .with_file(path, None)
            .build()?;
        let area = Rect::new(3, 2, 100, 24);
        let mut compositor = Compositor::new(area);
        compositor.push(Box::new(EditorView::new(Keymaps::new(Box::new(
            ArcSwap::from_pointee(keymap::default()),
        )))));
        let mut harness = Self {
            app,
            compositor,
            jobs,
            area,
            surface: Buffer::empty(area),
        };
        harness.render();
        Ok(harness)
    }

    fn render(&mut self) {
        let mut cx = Context {
            editor: &mut self.app.editor,
            jobs: &mut self.jobs,
            scroll: None,
        };
        self.compositor
            .render(self.area, &mut self.surface, &mut cx);
    }

    fn event(&mut self, event: Event) {
        let mut cx = Context {
            editor: &mut self.app.editor,
            jobs: &mut self.jobs,
            scroll: None,
        };
        self.compositor.handle_event(&event, &mut cx);
        self.drain_callbacks();
        self.render();
    }

    fn drain_callbacks(&mut self) {
        while let Ok(callback) = self.jobs.callbacks.try_recv() {
            if let Some(job) = self.jobs.handle_callback(
                &mut self.app.editor,
                &mut self.compositor,
                Ok(Some(callback)),
            ) {
                self.jobs.add(job);
            }
        }
    }

    fn panel_body(&self) -> Option<Rect> {
        let editor = self.app.editor.tree.area();
        (editor.bottom() < self.area.bottom() - 2).then(|| {
            Rect::new(
                editor.x,
                editor.bottom(),
                editor.width,
                self.area.bottom() - editor.bottom() - 2,
            )
        })
    }

    async fn wait_for_terminal_text(&mut self, text: &str) -> anyhow::Result<()> {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                self.drain_callbacks();
                self.render();
                let area = self.panel_body().unwrap_or_default();
                let rendered = (area.y..area.bottom())
                    .map(|y| {
                        (area.x..area.right())
                            .map(|x| self.surface.get(x, y).unwrap().symbol.as_str())
                            .collect::<String>()
                            .trim_end()
                            .to_owned()
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let rendered = format!("\n{rendered}\n");
                if rendered.contains(text) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| {
            let area = self.panel_body().unwrap_or_default();
            let screen = (area.y.saturating_sub(1)..area.bottom())
                .map(|y| {
                    (area.x..area.right())
                        .map(|x| self.surface.get(x, y).unwrap().symbol.as_str())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            anyhow::anyhow!("terminal did not render {text:?}; visible panel:\n{screen}")
        })
    }

    async fn assert_terminal_directory(&mut self, directory: &Path) -> anyhow::Result<()> {
        tokio::time::timeout(Duration::from_secs(10), async {
            while self.panel_body().is_none() {
                self.drain_callbacks();
                self.render();
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await?;
        self.keys("pwd<ret>")?;
        self.wait_for_terminal_text(&format!("\n{}\n", directory.display()))
            .await
    }

    fn session_tab(&self, number: u64) -> (u16, u16) {
        let y = self.panel_body().unwrap().y - 1;
        let prefix = format!("{number}:");
        let x = (self.area.x..self.area.right())
            .find(|&x| {
                let text: String = (x..self.area.right())
                    .map(|x| self.surface.get(x, y).unwrap().symbol.as_str())
                    .collect();
                text.starts_with(&prefix)
            })
            .expect("the terminal session tab should be visible");
        (x, y)
    }

    fn keys(&mut self, keys: &str) -> anyhow::Result<()> {
        for key in parse_macro(keys)? {
            self.event(Event::Key(key));
        }
        Ok(())
    }

    fn click(&mut self, button: MouseButton, column: u16, row: u16) {
        for kind in [MouseEventKind::Down(button), MouseEventKind::Up(button)] {
            self.event(Event::Mouse(MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::empty(),
            }));
        }
    }

    fn has_menu(&mut self) -> bool {
        self.compositor.layer_count() > 1
    }

    fn sidebar_row(&self, label: &str) -> u16 {
        (self.area.y + 1..self.area.bottom() - 1)
            .find(|&y| {
                let text: String = (self.area.x..self.area.x + 29)
                    .map(|x| self.surface.get(x, y).unwrap().symbol.as_str())
                    .collect();
                text.contains(label)
            })
            .unwrap_or_else(|| panic!("sidebar entry {label:?} should be visible"))
    }

    fn menu_item(&self, label: &str) -> (u16, u16) {
        for y in self.area.y..self.area.bottom() {
            for x in self.area.x..self.area.right() {
                let text: String = (x..self.area.right())
                    .map(|x| self.surface.get(x, y).unwrap().symbol.as_str())
                    .collect();
                if text.starts_with(label) {
                    return (x, y);
                }
            }
        }
        panic!("menu item {label:?} should be visible")
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn context_menu_terminal_uses_target_directory_and_command_entry() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let first = directory.path().join("first.txt");
    let folder = directory.path().join("folder");
    let child = folder.join("child.txt");
    std::fs::create_dir(&folder)?;
    std::fs::write(&first, "first\n")?;
    std::fs::write(&child, "child\n")?;
    let mut h = Harness::new(&first)?;
    h.app.editor.open(&child, Action::Load)?;
    h.render();

    let child_column = h.area.x + " first.txt ".width() as u16;
    h.click(MouseButton::Right, child_column, h.area.y);
    let (x, y) = h.menu_item("Open terminal");
    h.click(MouseButton::Left, x, y);
    assert_eq!(current_ref!(h.app.editor).1.path(), Some(first.as_path()));
    assert_eq!(h.app.editor.documents.len(), 2);
    assert!(h.panel_body().is_some());
    assert!(h.compositor.find::<Terminal>().is_none());
    h.assert_terminal_directory(&folder).await?;
    h.keys("<C-g>")?;
    assert_eq!(current_ref!(h.app.editor).1.path(), Some(first.as_path()));

    h.click(MouseButton::Right, h.area.x, h.area.y);
    h.keys("jjj<ret><esc>")?;
    let row = h.sidebar_row("folder");
    h.click(MouseButton::Right, h.area.x + 8, row);
    let (x, y) = h.menu_item("Open terminal");
    h.click(MouseButton::Left, x, y);
    h.assert_terminal_directory(&folder).await?;
    h.keys("<C-g>")?;

    let row = h.sidebar_row("folder");
    h.click(MouseButton::Left, h.area.x + 8, row);
    let row = h.sidebar_row("child.txt");
    h.click(MouseButton::Right, h.area.x + 8, row);
    let (x, y) = h.menu_item("Open terminal");
    h.click(MouseButton::Left, x, y);
    h.assert_terminal_directory(&folder).await?;
    h.keys("<C-g><esc>:terminal<ret>")?;
    h.assert_terminal_directory(directory.path()).await?;
    h.keys("<C-g>")?;
    assert!(h.panel_body().is_none());
    helpers::assert_status_not_error(&h.app.editor);
    Ok(())
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn terminal_panel_toggle_preserves_shell_and_editor_focus() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let first = directory.path().join("first.txt");
    let folder = directory.path().join("folder");
    std::fs::create_dir(&folder)?;
    std::fs::write(&first, "first\n")?;
    let mut h = Harness::new(&first)?;
    let first_id = current_ref!(h.app.editor).1.id();
    let original_height = h.app.editor.tree.area().height;
    h.keys("<C-/>")?;
    assert_eq!(current_ref!(h.app.editor).1.id(), first_id);
    assert_eq!(h.app.editor.documents.len(), 1);
    assert_eq!(h.app.editor.tree.views().count(), 1);
    assert!(h.app.editor.tree.area().height < original_height);
    h.assert_terminal_directory(directory.path()).await?;
    h.keys("cd folder<ret>")?;
    h.assert_terminal_directory(&folder).await?;
    h.keys("<C-/>")?;
    assert!(h.panel_body().is_none());
    assert_eq!(h.app.editor.tree.area().height, original_height);
    h.keys("iX<esc><C-_>")?;
    h.assert_terminal_directory(&folder).await?;
    let editor = h.app.editor.tree.area();
    h.click(MouseButton::Left, editor.x + 8, editor.y);
    h.keys("iY<esc>")?;
    assert!(h.panel_body().is_some());
    assert!(current_ref!(h.app.editor)
        .1
        .text()
        .to_string()
        .contains('Y'));
    let body = h.panel_body().unwrap();
    h.click(MouseButton::Left, body.x + 2, body.y + 1);
    h.assert_terminal_directory(&folder).await?;
    h.keys("<C-/>")?;
    h.keys("i<C-/>")?;
    assert!(h.panel_body().is_some());
    h.assert_terminal_directory(&folder).await?;
    h.keys("<C-g>")?;
    assert!(h.panel_body().is_none());
    assert_eq!(current_ref!(h.app.editor).1.id(), first_id);
    assert_eq!(h.app.editor.documents.len(), 1);
    h.keys("<C-/>")?;
    h.assert_terminal_directory(directory.path()).await?;
    let original_area = h.area;
    h.area = Rect::new(3, 2, 10, 4);
    h.surface = Buffer::empty(h.area);
    h.render();
    assert!(h.panel_body().is_none());
    h.area = original_area;
    h.surface = Buffer::empty(h.area);
    h.render();
    h.assert_terminal_directory(directory.path()).await?;
    h.keys("<C-g>")?;
    helpers::assert_status_not_error(&h.app.editor);
    Ok(())
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn terminal_panel_statusline_stays_below_terminal_and_tracks_file_focus() -> anyhow::Result<()>
{
    let directory = tempfile::tempdir()?;
    let first = directory.path().join("first.txt");
    let second = directory.path().join("second.txt");
    std::fs::write(&first, "first\n")?;
    std::fs::write(&second, "second\n")?;
    for sidebar in [false, true] {
        let mut h = Harness::new(&first)?;
        h.app.editor.open(&second, Action::Load)?;
        h.render();
        if sidebar {
            h.click(MouseButton::Right, h.area.x, h.area.y);
            h.keys("jjj<ret><esc>")?;
        }
        h.keys("<C-/>")?;
        h.assert_terminal_directory(directory.path()).await?;
        let body = h.panel_body().unwrap();
        let status_y = h.area.bottom() - 2;
        assert_eq!(body.bottom(), status_y);
        let row = |h: &Harness, y| -> String {
            (h.area.x..h.area.right())
                .map(|x| h.surface.get(x, y).unwrap().symbol.as_str())
                .collect()
        };
        assert!(row(&h, status_y).contains("first.txt"));
        assert!(row(&h, body.y - 1).contains("TERMINAL"));
        assert!(!row(&h, body.y - 1).contains("first.txt"));
        let second_tab = h.app.editor.tree.area().x + " first.txt ".width() as u16;
        h.click(MouseButton::Left, second_tab, h.area.y);
        assert_eq!(current_ref!(h.app.editor).1.path(), Some(second.as_path()));
        assert!(row(&h, status_y).contains("second.txt"));
        assert!(!row(&h, status_y).contains("first.txt"));
        h.click(MouseButton::Left, h.area.x + 2, status_y);
        assert_eq!(current_ref!(h.app.editor).1.path(), Some(second.as_path()));
        h.app.editor.set_status("FOOTER_MESSAGE");
        h.render();
        assert!(row(&h, h.area.bottom() - 1).contains("FOOTER_MESSAGE"));
        assert!(row(&h, status_y).contains("second.txt"));
        let body = h.panel_body().unwrap();
        h.click(MouseButton::Left, body.x + 2, body.y + 1);
        h.keys("<C-g>")?;
        assert!(h.panel_body().is_none());
        assert!(row(&h, status_y).contains("second.txt"));
        helpers::assert_status_not_error(&h.app.editor);
    }
    Ok(())
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn terminal_panel_sessions_resize_and_exit_independently() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let first = directory.path().join("first.txt");
    let folder = directory.path().join("folder");
    std::fs::create_dir(&folder)?;
    std::fs::write(&first, "first\n")?;
    let mut h = Harness::new(&first)?;
    h.keys("<C-/>")?;
    h.assert_terminal_directory(directory.path()).await?;
    h.keys("cd folder<ret>")?;
    h.assert_terminal_directory(&folder).await?;
    let header = h.panel_body().unwrap().y - 1;
    h.click(MouseButton::Left, h.area.right() - 11, header);
    h.assert_terminal_directory(directory.path()).await?;
    let (x, y) = h.session_tab(1);
    h.click(MouseButton::Left, x, y);
    h.assert_terminal_directory(&folder).await?;
    // Resize the dock without changing the editor's split structure.
    let header = h.panel_body().unwrap().y - 1;
    h.event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: h.area.x + 2,
        row: header,
        modifiers: KeyModifiers::empty(),
    }));
    h.event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: h.area.x + 2,
        row: header - 4,
        modifiers: KeyModifiers::empty(),
    }));
    h.event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column: h.area.x + 2,
        row: header - 4,
        modifiers: KeyModifiers::empty(),
    }));
    assert_eq!(h.panel_body().unwrap().y - 1, header - 4);
    let body = h.panel_body().unwrap();
    h.keys("echo SIZE_START;stty size;echo SIZE_END<ret>")?;
    h.wait_for_terminal_text(&format!(
        "SIZE_START\n{} {}\nSIZE_END",
        body.height, body.width
    ))
    .await?;
    h.keys("exit<ret>")?;
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            h.drain_callbacks();
            h.render();
            let header = h.panel_body().unwrap().y - 1;
            let text: String = (h.area.x..h.area.right())
                .map(|x| h.surface.get(x, header).unwrap().symbol.as_str())
                .collect();
            if !text.contains("1:") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    h.assert_terminal_directory(directory.path()).await?;
    assert_eq!(h.app.editor.documents.len(), 1);
    assert_eq!(h.app.editor.tree.views().count(), 1);
    h.keys("cd folder<ret>")?;
    h.assert_terminal_directory(&folder).await?;
    h.area = Rect::new(3, 2, 46, 24);
    h.surface = Buffer::empty(h.area);
    h.render();
    let header = h.panel_body().unwrap().y - 1;
    h.click(MouseButton::Left, h.area.right() - 11, header);
    h.assert_terminal_directory(directory.path()).await?;
    h.session_tab(3);
    h.event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: h.area.x + 2,
        row: header,
        modifiers: KeyModifiers::empty(),
    }));
    h.assert_terminal_directory(&folder).await?;
    h.keys("<C-g>")?;
    h.assert_terminal_directory(directory.path()).await?;
    let header = h.panel_body().unwrap().y - 1;
    h.click(MouseButton::Left, h.area.right() - 3, header);
    assert!(h.panel_body().is_none());
    h.keys("<C-/>")?;
    h.assert_terminal_directory(directory.path()).await?;
    let header = h.panel_body().unwrap().y - 1;
    h.click(MouseButton::Left, h.area.right() - 7, header);
    assert!(h.panel_body().is_none());
    helpers::assert_status_not_error(&h.app.editor);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn context_menu_buffer_targets_and_modified_buffer_protection() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let first = directory.path().join("first.txt");
    let second = directory.path().join("second.txt");
    std::fs::write(&first, "first\n")?;
    std::fs::write(&second, "second\n")?;
    let mut h = Harness::new(&first)?;
    let first_id = current_ref!(h.app.editor).1.id();
    let second_id = h.app.editor.open(&second, Action::Load)?;
    h.render();
    let second_column = h.area.x + " first.txt ".width() as u16;
    h.click(MouseButton::Right, second_column, h.area.y);
    assert!(h.has_menu());
    assert_eq!(current_ref!(h.app.editor).1.id(), first_id);
    h.menu_item("Close other buffers");
    assert_eq!(
        h.compositor.cursor(h.area, &h.app.editor).1,
        CursorKind::Hidden
    );
    h.keys("<esc>")?;
    assert!(!h.has_menu());

    h.click(MouseButton::Right, second_column, h.area.y);
    h.keys("jjjj<ret>")?;
    let copied: Vec<_> = h
        .app
        .editor
        .registers
        .read('+', &h.app.editor)
        .unwrap()
        .map(|value| value.into_owned())
        .collect();
    assert_eq!(copied, vec![second.to_string_lossy().into_owned()]);
    assert_eq!(current_ref!(h.app.editor).1.id(), first_id);

    h.click(MouseButton::Right, second_column, h.area.y);
    h.event(Event::FocusLost);
    assert!(!h.has_menu());
    h.event(Event::FocusGained);

    let selection = {
        let (view, doc) = current_ref!(h.app.editor);
        doc.selection(view.id).clone()
    };
    h.click(MouseButton::Right, second_column, h.area.y);
    h.click(MouseButton::Right, h.area.x + 8, h.area.y + 1);
    assert!(!h.has_menu());
    let (view, doc) = current_ref!(h.app.editor);
    assert_eq!(doc.selection(view.id), &selection);

    h.click(MouseButton::Right, second_column, h.area.y);
    // An outside click dismisses the menu without activating the underlying tab.
    h.click(MouseButton::Left, h.area.x + 1, h.area.y);
    assert!(!h.has_menu());
    assert_eq!(current_ref!(h.app.editor).1.id(), first_id);

    h.click(MouseButton::Right, second_column, h.area.y);
    let (x, y) = h.menu_item("Close");
    h.click(MouseButton::Left, x, y);
    assert!(h.app.editor.document(second_id).is_none());
    assert_eq!(current_ref!(h.app.editor).1.id(), first_id);

    let second_id = h.app.editor.open(&second, Action::Replace)?;
    h.keys("iX<esc>")?;
    h.app.editor.switch(first_id, Action::Replace);
    h.render();
    h.click(MouseButton::Right, second_column, h.area.y);
    h.keys("j<ret>")?;
    assert!(h.app.editor.document(second_id).unwrap().is_modified());
    assert!(h
        .app
        .editor
        .status_msg
        .as_ref()
        .unwrap()
        .0
        .contains("unsaved"));
    assert_eq!(std::fs::read_to_string(&second)?, "second\n");

    h.click(MouseButton::Right, second_column, h.area.y);
    h.keys("<ret>")?;
    h.jobs
        .finish(&mut h.app.editor, Some(&mut h.compositor))
        .await?;
    h.app.editor.flush_writes().await?;
    assert_eq!(std::fs::read_to_string(&second)?, "Xsecond\n");
    assert!(!h.app.editor.document(second_id).unwrap().is_modified());

    h.click(MouseButton::Right, h.area.x, h.area.y);
    h.keys("jj<ret>")?;
    assert!(h.app.editor.document(first_id).is_some());
    assert!(h.app.editor.document(second_id).is_none());
    helpers::assert_status_not_error(&h.app.editor);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn context_menu_explorer_rename_split_and_confirm_delete() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let first = directory.path().join("first.txt");
    let second = directory.path().join("second.txt");
    let renamed = directory.path().join("renamed.txt");
    std::fs::write(&first, "first\n")?;
    std::fs::write(&second, "second\n")?;
    let mut h = Harness::new(&first)?;
    h.click(MouseButton::Right, h.area.x, h.area.y);
    h.keys("jjj<ret><esc>")?;
    let row = h.sidebar_row("second.txt");
    h.click(MouseButton::Right, h.area.x + 8, row);
    assert!(h.has_menu());
    assert_eq!(current_ref!(h.app.editor).1.path(), Some(first.as_path()));
    let (x, y) = h.menu_item("Rename");
    h.click(MouseButton::Left, x, y);
    h.keys("<C-u>renamed.txt<ret>")?;
    assert!(renamed.exists());
    assert!(!second.exists());
    assert_eq!(current_ref!(h.app.editor).1.path(), Some(first.as_path()));

    let row = h.sidebar_row("renamed.txt");
    h.click(MouseButton::Right, h.area.x + 8, row);
    h.keys("jj<ret>")?;
    assert_eq!(h.app.editor.tree.views().count(), 2);
    assert_eq!(current_ref!(h.app.editor).1.path(), Some(renamed.as_path()));

    let row = h.sidebar_row("renamed.txt");
    h.click(MouseButton::Right, h.area.x + 8, row);
    let (x, y) = h.menu_item("Delete");
    h.click(MouseButton::Left, x, y);
    assert!(renamed.exists());
    h.keys("n<ret>")?;
    assert!(renamed.exists());
    h.click(MouseButton::Right, h.area.x + 8, row);
    h.keys("jjjj<ret>y<ret>")?;
    assert!(!renamed.exists());
    helpers::assert_status_not_error(&h.app.editor);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn context_menu_directory_creation_and_buffer_reveal() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let first = directory.path().join("first.txt");
    let folder = directory.path().join("folder");
    std::fs::create_dir(&folder)?;
    std::fs::write(&first, "first\n")?;
    let mut h = Harness::new(&first)?;
    h.click(MouseButton::Right, h.area.x, h.area.y);
    h.keys("jjj<ret>")?;
    assert_eq!(current_ref!(h.app.editor).0.area.x, h.area.x + 30);
    let row = h.sidebar_row("folder");
    h.click(MouseButton::Right, h.area.x + 8, row);
    h.keys("j<ret>nested<ret>")?;
    assert!(folder.join("nested").is_dir());

    let row = h.sidebar_row("folder");
    h.click(MouseButton::Right, h.area.x + 8, row);
    h.keys("<ret>child.txt<ret>")?;
    assert!(folder.join("child.txt").is_file());
    assert_eq!(current_ref!(h.app.editor).1.path(), Some(first.as_path()));

    // Menus remain inside a small viewport and keyboard navigation scrolls their rows.
    h.keys("<esc>")?;
    h.area = Rect::new(3, 2, 25, 5);
    h.surface = Buffer::empty(h.area);
    h.render();
    h.click(MouseButton::Right, h.area.x, h.area.y);
    assert!(h.has_menu());
    h.keys("jjj")?;
    h.menu_item("Reveal in file explor");
    h.event(Event::Resize(20, 5));
    assert!(!h.has_menu());
    helpers::assert_status_not_error(&h.app.editor);
    Ok(())
}
