use std::path::{Path, PathBuf};

use helix_core::{unicode::width::UnicodeWidthStr, Position};
use helix_view::{
    graphics::{CursorKind, Rect},
    input::{MouseButton, MouseEventKind},
    Editor,
};
use tui::buffer::Buffer;

use super::terminal::{Terminal, TerminalId};
use crate::compositor::{Component, Context, Event, EventResult};

struct Session {
    id: TerminalId,
    directory: PathBuf,
    terminal: Terminal,
}

/// A docked terminal panel, independent of file buffers and editor splits.
#[derive(Default)]
pub(super) struct TerminalPanel {
    sessions: Vec<Session>,
    active: Option<TerminalId>,
    next_id: u64,
    visible: bool,
    focused: bool,
    viewport: Rect,
    area: Rect,
    height: Option<u16>,
    dragging: bool,
    consume_release: bool,
    tabs: Vec<(Rect, TerminalId)>,
}

impl TerminalPanel {
    pub fn session_mut(&mut self, id: TerminalId) -> Option<&mut Terminal> {
        self.sessions
            .iter_mut()
            .find(|session| session.id == id)
            .map(|session| &mut session.terminal)
    }

    pub fn remove(&mut self, id: TerminalId) {
        let Some(index) = self.sessions.iter().position(|session| session.id == id) else {
            return;
        };
        self.sessions.remove(index);
        if self.active == Some(id) {
            self.active = self
                .sessions
                .get(index.min(self.sessions.len().saturating_sub(1)))
                .map(|session| session.id);
        }
        if self.sessions.is_empty() {
            self.hide();
        }
    }

    fn hide(&mut self) {
        self.visible = false;
        self.focused = false;
        self.dragging = false;
    }

    pub fn is_visible(&self) -> bool {
        self.visible && self.area.height > 1
    }

    pub fn is_focused(&self) -> bool {
        self.is_visible() && self.focused
    }

    pub fn layout(&mut self, viewport: Rect) -> Rect {
        self.viewport = viewport;
        if !self.visible || viewport.height < 7 || viewport.width < 12 {
            self.area = Rect::default();
            return viewport;
        }
        // Reserve the global statusline below the panel. The panel header shares
        // the bottom editor views' statusline row, keeping their text area intact.
        let content = viewport.clip_bottom(1);
        let height = self
            .height
            .unwrap_or((viewport.height / 3).max(5))
            .clamp(3, content.height - 3);
        self.area = Rect::new(content.x, content.bottom() - height, content.width, height);
        content.clip_bottom(height - 1)
    }

    pub fn open(
        &mut self,
        directory: &Path,
        fallback: Rect,
        editor: &mut Editor,
    ) -> anyhow::Result<()> {
        let previous = (self.visible, self.focused);
        self.visible = true;
        let viewport = if self.viewport.width > 0 {
            self.viewport
        } else {
            fallback.clip_bottom(1)
        };
        self.layout(viewport);
        let id = TerminalId(self.next_id);
        let terminal = match Terminal::for_panel(
            directory,
            id,
            self.area.clip_top(1),
            editor.theme.get("ui.background").bg,
        ) {
            Ok(terminal) => terminal,
            Err(err) => {
                (self.visible, self.focused) = previous;
                return Err(err);
            }
        };
        self.next_id += 1;
        self.sessions.push(Session {
            id,
            directory: directory.to_path_buf(),
            terminal,
        });
        self.active = Some(id);
        self.focused = true;
        Ok(())
    }

    pub fn toggle(&mut self, fallback: Rect, editor: &mut Editor) -> anyhow::Result<()> {
        if self.visible {
            self.hide();
        } else if self.sessions.is_empty() {
            let directory = helix_view::doc!(editor)
                .path()
                .and_then(Path::parent)
                .map(ToOwned::to_owned)
                .unwrap_or_else(helix_stdx::env::current_working_dir);
            self.open(&directory, fallback, editor)?;
        } else {
            self.visible = true;
            self.focused = true;
        }
        Ok(())
    }

    fn control(&self, column: u16) -> Option<u16> {
        (column >= self.area.right().saturating_sub(12))
            .then(|| (column - self.area.right().saturating_sub(12)) / 4)
    }
}

impl Component for TerminalPanel {
    fn handle_event(&mut self, event: &Event, cx: &mut Context) -> EventResult {
        if self.consume_release
            && matches!(event, Event::Mouse(mouse) if mouse.kind == MouseEventKind::Up(MouseButton::Left))
        {
            self.consume_release = false;
            self.dragging = false;
            return EventResult::Consumed(None);
        }
        if !self.visible || self.area.height < 2 {
            return EventResult::Ignored(None);
        }
        match event {
            Event::Mouse(mouse) => {
                if self.dragging {
                    match mouse.kind {
                        MouseEventKind::Drag(MouseButton::Left) => {
                            self.height = Some(
                                self.viewport
                                    .bottom()
                                    .saturating_sub(1)
                                    .saturating_sub(mouse.row)
                                    .clamp(3, self.viewport.height.saturating_sub(4).max(3)),
                            );
                            return EventResult::Consumed(None);
                        }
                        MouseEventKind::Up(MouseButton::Left) => {
                            self.dragging = false;
                            return EventResult::Consumed(None);
                        }
                        _ => {}
                    }
                }
                let inside = mouse.column >= self.area.x
                    && mouse.column < self.area.right()
                    && mouse.row >= self.area.y
                    && mouse.row < self.area.bottom();
                if !inside {
                    if matches!(mouse.kind, MouseEventKind::Down(_)) {
                        self.focused = false;
                    }
                    return EventResult::Ignored(None);
                }
                if matches!(mouse.kind, MouseEventKind::Down(_)) {
                    self.focused = true;
                }
                if mouse.row == self.area.y {
                    if matches!(
                        mouse.kind,
                        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                    ) {
                        if let Some(index) = self
                            .sessions
                            .iter()
                            .position(|session| Some(session.id) == self.active)
                        {
                            let index = if mouse.kind == MouseEventKind::ScrollUp {
                                index.saturating_sub(1)
                            } else {
                                (index + 1).min(self.sessions.len() - 1)
                            };
                            self.active = Some(self.sessions[index].id);
                        }
                    }
                    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                        self.consume_release = true;
                        match self.control(mouse.column) {
                            Some(0) => {
                                let directory = self
                                    .sessions
                                    .iter()
                                    .find(|session| Some(session.id) == self.active)
                                    .map(|session| session.directory.clone())
                                    .unwrap_or_else(helix_stdx::env::current_working_dir);
                                if let Err(err) = self.open(&directory, self.viewport, cx.editor) {
                                    cx.editor.set_error(format!("{err:#}"));
                                }
                            }
                            Some(1) => {
                                if let Some(id) = self.active {
                                    self.remove(id);
                                }
                            }
                            Some(2) => self.hide(),
                            _ => {
                                if let Some((_, id)) = self.tabs.iter().find(|(area, _)| {
                                    mouse.column >= area.x && mouse.column < area.right()
                                }) {
                                    self.active = Some(*id);
                                } else {
                                    self.dragging = true;
                                }
                            }
                        }
                    }
                    return EventResult::Consumed(None);
                }
            }
            Event::Key(_) | Event::Paste(_) if !self.focused => return EventResult::Ignored(None),
            Event::Resize(..) | Event::FocusLost => {
                self.dragging = false;
                return EventResult::Ignored(None);
            }
            Event::Key(_) | Event::Paste(_) => {}
            _ => return EventResult::Ignored(None),
        }
        if let Some(id) = self.active {
            let area = self.area.clip_top(1);
            if let Some(terminal) = self.session_mut(id) {
                terminal.resize(area, cx.editor);
                return terminal.handle_event(event, cx);
            }
        }
        EventResult::Consumed(None)
    }

    fn render(&mut self, _area: Rect, surface: &mut Buffer, cx: &mut Context) {
        self.tabs.clear();
        if !self.visible || self.area.height < 2 {
            return;
        }
        let header = self.area.with_height(1);
        let style = cx.editor.theme.get(if self.focused {
            "ui.statusline"
        } else {
            "ui.statusline.inactive"
        });
        surface.clear_with(header, style);
        let controls = self.area.right() - 12;
        let mut x = surface
            .set_stringn(
                header.x,
                header.y,
                " TERMINAL ",
                usize::from(controls.saturating_sub(header.x)),
                style,
            )
            .0;
        let labels: Vec<_> = self
            .sessions
            .iter()
            .map(|session| {
                let name = session
                    .directory
                    .file_name()
                    .unwrap_or(session.directory.as_os_str())
                    .to_string_lossy();
                format!(" {}:{} ", session.id.0 + 1, name)
            })
            .collect();
        let mut start = self
            .sessions
            .iter()
            .position(|session| Some(session.id) == self.active)
            .unwrap_or(0);
        let mut used = labels
            .get(start)
            .map_or(0, |label| label.width())
            .min(usize::from(controls.saturating_sub(x)));
        while start > 0
            && used + labels[start - 1].width() <= usize::from(controls.saturating_sub(x))
        {
            start -= 1;
            used += labels[start].width();
        }
        for (session, label) in self.sessions.iter().zip(&labels).skip(start) {
            if x >= controls {
                break;
            }
            let width = label.width().min(usize::from(controls - x)) as u16;
            let tab_style = if Some(session.id) == self.active {
                cx.editor.theme.get("ui.menu.selected")
            } else {
                style
            };
            let end = surface
                .set_stringn(x, header.y, label, usize::from(width), tab_style)
                .0;
            self.tabs
                .push((Rect::new(x, header.y, end - x, 1), session.id));
            x = end;
        }
        surface.set_stringn(controls, header.y, "[+] [×] [−] ", 12, style);
        if let Some(id) = self.active {
            let area = self.area.clip_top(1);
            if let Some(terminal) = self.session_mut(id) {
                terminal.render_in_area(area, surface, cx);
            }
        }
    }

    fn cursor(&self, area: Rect, editor: &Editor) -> (Option<Position>, CursorKind) {
        if self.is_focused() {
            if let Some(session) = self
                .sessions
                .iter()
                .find(|session| Some(session.id) == self.active)
            {
                return session.terminal.cursor(area, editor);
            }
        }
        (None, CursorKind::Hidden)
    }
}
