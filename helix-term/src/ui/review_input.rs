use crate::{
    alt,
    compositor::{Component, Context, Event, EventResult},
    ctrl, key, shift,
    ui::{completers, Prompt},
};
use helix_core::Position;
use helix_view::{
    graphics::{CursorKind, Rect},
    DocumentId, Editor, ViewId,
};
use tui::buffer::Buffer;

/// Capture input while the document formatter lays out and paints the draft in its frame.
pub(crate) struct ReviewInput {
    doc: DocumentId,
    view: ViewId,
    input: Prompt,
}

impl ReviewInput {
    pub fn new(editor: &Editor, doc: DocumentId, view: ViewId) -> Self {
        let text = editor.documents[&doc].review_input().unwrap();
        let input = Prompt::new("".into(), None, completers::none, |_, _, _| {})
            .with_line(text.to_owned(), editor);
        Self { doc, view, input }
    }

    fn close(&self, cx: &mut Context, save: bool) -> EventResult {
        if let Some(doc) = cx.editor.document_mut(self.doc) {
            doc.finish_review_edit(save);
        }
        if cx
            .editor
            .tree
            .try_get(self.view)
            .is_some_and(|v| v.doc == self.doc)
        {
            cx.editor.ensure_cursor_in_view(self.view);
        }
        EventResult::Consumed(Some(Box::new(|compositor, _| {
            compositor.pop();
        })))
    }
}

impl Component for ReviewInput {
    fn handle_event(&mut self, event: &Event, cx: &mut Context) -> EventResult {
        if cx.editor.tree.focus != self.view
            || cx
                .editor
                .tree
                .try_get(self.view)
                .is_none_or(|v| v.doc != self.doc)
        {
            return self.close(cx, false);
        }
        match event {
            Event::Key(key!(Esc) | ctrl!('c')) => return self.close(cx, false),
            Event::Key(key!(Enter)) => {
                if self.input.line().trim().is_empty() {
                    cx.editor
                        .set_error("Enter a comment or press Esc to cancel");
                    return EventResult::Consumed(None);
                }
                return self.close(cx, true);
            }
            Event::Key(key!(Up) | key!(Down)) => {
                let (view, doc) = current_ref!(cx.editor);
                let down = matches!(event, Event::Key(key!(Down)));
                if let Some(offset) =
                    doc.vertical_review_input_offset(view.id, view.inner_width(doc), down)
                {
                    self.input.set_position(offset);
                }
            }
            Event::Key(shift!(Enter) | alt!(Enter)) => self.input.insert_str("\n", cx.editor),
            Event::Paste(text) => self
                .input
                .insert_str(&text.replace("\r\n", "\n").replace('\r', "\n"), cx.editor),
            Event::Key(_) => {
                self.input.handle_event(event, cx);
            }
            Event::Mouse(_) => return EventResult::Consumed(None),
            _ => return EventResult::Ignored(None),
        }
        let doc = cx.editor.document_mut(self.doc).unwrap();
        doc.update_review_input(self.input.line(), self.input.position());
        cx.editor.ensure_cursor_in_view(self.view);
        EventResult::Consumed(None)
    }

    fn render(&mut self, _: Rect, _: &mut Buffer, _: &mut Context) {}

    fn cursor(&self, _: Rect, editor: &Editor) -> (Option<Position>, CursorKind) {
        (editor.cursor().0, CursorKind::Bar)
    }
}
