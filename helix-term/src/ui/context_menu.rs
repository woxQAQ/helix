use crate::{
    compositor::{Callback, Component, Context, Event, EventResult},
    key,
};
use helix_core::{unicode::width::UnicodeWidthStr, Position};
use helix_view::{
    graphics::{CursorKind, Rect},
    input::{MouseButton, MouseEventKind},
    Editor,
};
use tui::{
    buffer::Buffer,
    widgets::{Block, Borders, Widget},
};

pub(super) struct MenuItem {
    label: &'static str,
    action: Option<Callback>,
}

impl MenuItem {
    pub fn new(label: &'static str, action: Callback) -> Self {
        Self {
            label,
            action: Some(action),
        }
    }
}

enum MenuPress {
    Item(usize),
    Outside,
}

/// A menu anchored to a mouse position, with actions bound to the clicked target.
pub(super) struct ContextMenu {
    position: Position,
    items: Vec<MenuItem>,
    selected: usize,
    press: Option<MenuPress>,
    offset: usize,
    area: Rect,
}

impl ContextMenu {
    pub const ID: &'static str = "context-menu";

    pub fn new(position: Position, items: Vec<MenuItem>) -> Self {
        Self {
            position,
            items,
            selected: 0,
            press: None,
            offset: 0,
            area: Rect::default(),
        }
    }

    fn finish(&mut self, index: Option<usize>) -> EventResult {
        let action = index.and_then(|index| self.items.get_mut(index)?.action.take());
        EventResult::Consumed(Some(Box::new(move |compositor, cx| {
            compositor.remove(Self::ID);
            if let Some(action) = action {
                action(compositor, cx);
            }
        })))
    }

    fn move_selection(&mut self, amount: isize) {
        self.selected = self
            .selected
            .saturating_add_signed(amount)
            .min(self.items.len().saturating_sub(1));
    }
}

impl Component for ContextMenu {
    fn handle_event(&mut self, event: &Event, _cx: &mut Context) -> EventResult {
        match event {
            Event::Key(key) => match *key {
                key!(Esc) => return self.finish(None),
                key!(Enter) => return self.finish(Some(self.selected)),
                key!(Up) | key!('k') => self.move_selection(-1),
                key!(Down) | key!('j') => self.move_selection(1),
                _ => {}
            },
            Event::Mouse(mouse) => {
                let inside = mouse.column >= self.area.x
                    && mouse.column < self.area.right()
                    && mouse.row >= self.area.y
                    && mouse.row < self.area.bottom();
                let hovered = (inside
                    && mouse.row > self.area.y
                    && mouse.row < self.area.bottom().saturating_sub(1)
                    && mouse.column > self.area.x
                    && mouse.column < self.area.right().saturating_sub(1))
                .then(|| self.offset + usize::from(mouse.row - self.area.y - 1))
                .filter(|&index| index < self.items.len());
                match mouse.kind {
                    MouseEventKind::Down(_) if !inside => self.press = Some(MenuPress::Outside),
                    MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Moved => {
                        if let Some(index) = hovered {
                            self.selected = index;
                        }
                        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                            self.press = hovered.map(MenuPress::Item);
                        }
                    }
                    MouseEventKind::Up(button) => match self.press.take() {
                        Some(MenuPress::Outside) => return self.finish(None),
                        Some(MenuPress::Item(index))
                            if button == MouseButton::Left && hovered == Some(index) =>
                        {
                            return self.finish(Some(index));
                        }
                        _ => {}
                    },
                    MouseEventKind::ScrollUp => self.move_selection(-1),
                    MouseEventKind::ScrollDown => self.move_selection(1),
                    MouseEventKind::Down(_) => self.press = None,
                    _ => {}
                }
            }
            Event::Resize(..) | Event::FocusLost => {
                let EventResult::Consumed(callback) = self.finish(None) else {
                    unreachable!()
                };
                return EventResult::Ignored(callback);
            }
            Event::IdleTimeout | Event::FocusGained => return EventResult::Ignored(None),
            _ => {}
        }
        EventResult::Consumed(None)
    }

    fn render(&mut self, viewport: Rect, surface: &mut Buffer, cx: &mut Context) {
        let width = self
            .items
            .iter()
            .map(|item| item.label.width())
            .max()
            .unwrap_or(0)
            .saturating_add(4)
            .min(usize::from(viewport.width)) as u16;
        let height = self
            .items
            .len()
            .saturating_add(2)
            .min(usize::from(viewport.height)) as u16;
        let x = self
            .position
            .col
            .min(usize::from(viewport.right().saturating_sub(width))) as u16;
        let y = self
            .position
            .row
            .min(usize::from(viewport.bottom().saturating_sub(height))) as u16;
        self.area = Rect::new(x.max(viewport.x), y.max(viewport.y), width, height);
        let style = cx.editor.theme.get("ui.menu");
        surface.clear_with(self.area, style);
        Block::default()
            .borders(Borders::ALL)
            .style(style)
            .render(self.area, surface);
        let rows = usize::from(height.saturating_sub(2));
        if rows == 0 || width < 4 {
            return;
        }
        self.offset = self.offset.min(self.selected);
        if self.selected >= self.offset + rows {
            self.offset = self.selected + 1 - rows;
        }
        for (row, item) in self.items.iter().skip(self.offset).take(rows).enumerate() {
            let area = Rect::new(self.area.x + 1, self.area.y + 1 + row as u16, width - 2, 1);
            let style = if self.offset + row == self.selected {
                cx.editor.theme.get("ui.menu.selected")
            } else {
                style
            };
            surface.set_style(area, style);
            surface.set_stringn(
                area.x + 1,
                area.y,
                item.label,
                usize::from(width - 4),
                style,
            );
        }
    }

    fn cursor(&self, _area: Rect, _editor: &Editor) -> (Option<Position>, CursorKind) {
        (
            Some(Position::new(self.area.y.into(), self.area.x.into())),
            CursorKind::Hidden,
        )
    }

    fn id(&self) -> Option<&'static str> {
        Some(Self::ID)
    }
}
