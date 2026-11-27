use std::sync::Arc;

use helix_view::{current_ref, document::Document, graphics::Rect, DocumentId, Editor};
use tui::{
    buffer::Buffer as Surface,
    widgets::{Paragraph, Widget, Wrap},
};

use super::markdown::{Markdown, RenderedMarkdown};
/// The pane stays hidden below this width, where wrapped prose no longer reads well.
const MIN_PREVIEW_WIDTH: u16 = 20;
/// The source views keep at least this many columns, so previewing never squeezes editing out.
const MIN_EDITOR_WIDTH: u16 = 30;

/// A read-only pane rendering the focused buffer as markdown.
///
/// The pane never takes focus: it scrolls to the line under the cursor instead, so the preview
/// follows the buffer without claiming keys or mouse events. `EditorView` owns the pane and decides
/// when it is shown; this type only reserves its columns and paints it.
#[derive(Default)]
pub struct MarkdownPreview {
    area: Rect,
    cache: Option<CachedRender>,
}

/// A rendered document together with the document state it was rendered from.
///
/// Rendering markdown and highlighting its code blocks is expensive enough that it should not run
/// on every frame, so the result is reused until the document, the theme, or the pane width
/// changes. `rows` maps rendered lines to screen rows: a wrapped line can occupy several rows, so
/// scrolling by rendered line would drift away from the cursor in wrapped documents.
struct CachedRender {
    doc: DocumentId,
    version: i32,
    theme: String,
    width: u16,
    rendered: RenderedMarkdown,
    rows: Vec<usize>,
}

impl MarkdownPreview {
    /// Reserves `width` columns on the right of `area` for the pane.
    ///
    /// Returns the area left for the editor views. The pane collapses to nothing when the viewport
    /// is too small for both sides; it keeps its toggled state so it reappears on resize.
    pub fn layout(&mut self, area: Rect, width: u16) -> Rect {
        let width = width.min(area.width.saturating_sub(MIN_EDITOR_WIDTH));
        if width < MIN_PREVIEW_WIDTH || area.height < 3 {
            self.area = Rect::default();
            return area;
        }
        self.area = Rect::new(area.right() - width, area.y, width, area.height);
        area.clip_right(width)
    }

    pub fn render(&mut self, surface: &mut Surface, editor: &Editor) {
        let area = self.area;
        if area.width < 2 || area.height < 2 {
            return;
        }

        let theme = &editor.theme;
        let (view, doc) = current_ref!(editor);

        // The column between the source views and the pane carries their window border.
        let header = area.clip_left(1).with_height(1);
        let style = theme.get("ui.statusline.inactive");
        surface.clear_with(header, style);
        let title = match doc.path().and_then(|path| path.file_name()) {
            Some(name) => format!(" PREVIEW {} ", name.to_string_lossy()),
            None => " PREVIEW ".to_string(),
        };
        surface.set_stringn(header.x, header.y, &title, usize::from(header.width), style);

        let content = area.clip_left(1).clip_top(1);
        surface.clear_with(content, theme.get("ui.background"));

        let cache = self.cached(doc, editor, content.width);
        let cursor_line = doc
            .selection(view.id)
            .primary()
            .cursor_line(doc.text().slice(..));
        let total_rows = cache.rows.last().copied().unwrap_or(0);
        let scroll = cache.rows[cache.rendered.rendered_line_of(cursor_line)]
            .min(total_rows.saturating_sub(usize::from(content.height)));

        let paragraph = Paragraph::new(&cache.rendered.text)
            .wrap(Wrap { trim: false })
            .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0));
        paragraph.render(content, surface);
    }

    fn cached(&mut self, doc: &Document, editor: &Editor, width: u16) -> &CachedRender {
        let theme = editor.theme.name();
        let stale = match &self.cache {
            Some(cache) => {
                cache.doc != doc.id()
                    || cache.version != doc.version()
                    || cache.theme != theme
                    || cache.width != width
            }
            None => true,
        };

        if stale {
            // The parser borrows the source text while rendering, so the buffer is copied once per
            // change. The rendered text itself is owned, which is what makes caching it possible.
            let contents = doc.text().to_string();
            let rendered = Markdown::new(contents, Arc::clone(&editor.syn_loader))
                .render_document(Some(&editor.theme));
            let rows = rows_per_line(&rendered.text, width);
            self.cache = Some(CachedRender {
                doc: doc.id(),
                version: doc.version(),
                theme: theme.to_string(),
                width,
                rendered,
                rows,
            });
        }

        self.cache.as_ref().unwrap()
    }
}

/// Start row of every rendered line at `width`, plus the total row count.
///
/// Each line of a `Text` ends a paragraph on its own (the paragraph widget feeds a `\n` grapheme
/// between lines), so wrapping every line in isolation yields the same rows as wrapping the whole
/// document.
fn rows_per_line(text: &tui::text::Text, width: u16) -> Vec<usize> {
    let mut rows = Vec::with_capacity(text.lines.len() + 1);
    let mut start = 0;
    rows.push(start);
    for line in &text.lines {
        let single = tui::text::Text::from(vec![line.clone()]);
        let paragraph = Paragraph::new(&single).wrap(Wrap { trim: false });
        start += usize::from(paragraph.required_size(width).1);
        rows.push(start);
    }
    rows
}
