use super::*;

use arc_swap::ArcSwap;
use helix_core::unicode::width::UnicodeWidthStr;
use helix_term::{
    application::Application,
    compositor::{Compositor, Context, Event},
    job::Jobs,
    keymap::{self, Keymaps},
    ui::EditorView,
};
use helix_view::{
    current_ref,
    editor::{Action, BufferLine},
    graphics::{CursorKind, Rect},
    input::{parse_macro, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
};
use std::path::Path;
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
        while let Ok(callback) = cx.jobs.callbacks.try_recv() {
            if let Some(job) =
                cx.jobs
                    .handle_callback(cx.editor, &mut self.compositor, Ok(Some(callback)))
            {
                cx.jobs.add(job);
            }
        }
        self.render();
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
