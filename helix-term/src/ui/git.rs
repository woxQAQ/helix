use std::{path::PathBuf, process::Stdio, time::Duration};

use helix_core::text_annotations::InlineAnnotation;
use helix_view::{current_ref, DocumentId, Editor};
use tokio::{io::AsyncWriteExt, process::Command};

use crate::{
    commands,
    job::{Callback, Jobs},
};

/// A request identifies the exact buffer snapshot whose line was blamed.
#[derive(Clone, PartialEq, Eq)]
struct BlameRequest {
    document: DocumentId,
    version: i32,
    line: usize,
    path: PathBuf,
}

impl BlameRequest {
    fn current(editor: &Editor) -> Option<Self> {
        let (view, doc) = current_ref!(editor);
        Some(Self {
            document: doc.id(),
            version: doc.version(),
            line: doc
                .selection(view.id)
                .primary()
                .cursor_line(doc.text().slice(..)),
            path: doc.path()?.to_owned(),
        })
    }
}

#[derive(Default)]
pub(super) struct InlineBlame {
    enabled: bool,
    generation: u64,
    request: Option<BlameRequest>,
    annotation: Vec<InlineAnnotation>,
}

impl InlineBlame {
    pub fn toggle(&mut self, editor: &mut Editor, jobs: &mut Jobs) {
        self.enabled = !self.enabled;
        self.generation += 1;
        self.request = None;
        self.annotation.clear();
        if self.enabled {
            self.refresh(editor, jobs);
        }
    }

    pub fn annotations(&self, editor: &Editor) -> &[InlineAnnotation] {
        if self.enabled && self.request.is_some() && self.request == BlameRequest::current(editor) {
            &self.annotation
        } else {
            &[]
        }
    }

    pub fn refresh(&mut self, editor: &mut Editor, jobs: &mut Jobs) {
        if !self.enabled {
            return;
        }
        let Some(request) = BlameRequest::current(editor) else {
            if self.request.take().is_some() {
                self.generation += 1;
            }
            self.annotation.clear();
            return;
        };
        if self.request.as_ref() == Some(&request) {
            return;
        }
        self.request = Some(request.clone());
        self.annotation.clear();
        self.generation += 1;
        let generation = self.generation;
        let (_, doc) = current_ref!(editor);
        let text = doc.text().to_string();
        let anchor = doc.text().line_to_char(request.line)
            + helix_core::line_ending::line_without_line_ending(
                &doc.text().slice(..),
                request.line,
            )
            .len_chars();
        jobs.callback(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(5),
                blame_line(&request.path, request.line, text),
            )
            .await;
            Ok(Callback::EditorCompositor(Box::new(
                move |editor, compositor| {
                    let Some(view) = compositor.find::<super::EditorView>() else {
                        return;
                    };
                    let blame = &mut view.inline_blame;
                    if !blame.enabled || blame.generation != generation {
                        return;
                    }
                    match result {
                        Ok(Ok(message)) => {
                            blame.annotation =
                                vec![InlineAnnotation::new(anchor, format!("  {message}"))];
                        }
                        Ok(Err(err)) => {
                            if BlameRequest::current(editor).as_ref() == Some(&request) {
                                editor.set_status(format!("Git blame: {err}"));
                            }
                        }
                        Err(_) => {
                            blame.request = None;
                            editor.set_status("Git blame timed out");
                        }
                    }
                },
            )))
        });
    }
}

async fn blame_line(path: &std::path::Path, line: usize, text: String) -> anyhow::Result<String> {
    if line >= text.lines().count() {
        return Ok("Not committed yet".into());
    }
    let mut child = Command::new("git")
        .arg("--no-pager")
        .arg("-C")
        .arg(path.parent().unwrap())
        .args(["blame", "--line-porcelain", "--contents", "-", "-L"])
        .arg(format!("{0},{0}", line + 1))
        .arg("--")
        .arg(path.file_name().unwrap())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut stdin = child.stdin.take().unwrap();
    let write = async move {
        stdin.write_all(text.as_bytes()).await?;
        stdin.shutdown().await
    };
    let (output, _) = tokio::join!(child.wait_with_output(), write);
    let output = output?;
    anyhow::ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let output = String::from_utf8(output.stdout)?;
    let hash = output.split_whitespace().next().unwrap_or_default();
    if hash.chars().all(|ch| ch == '0') {
        return Ok("Not committed yet".into());
    }
    let field = |prefix| {
        output
            .lines()
            .find_map(|line| line.strip_prefix(prefix))
            .unwrap_or_default()
    };
    Ok(format!(
        "{} · {} · {}",
        &hash[..hash.len().min(8)],
        field("author "),
        field("summary ")
    ))
}

/// Draw at the line end without changing wrapping or document coordinates.
pub(super) struct BlameDecoration<'a> {
    annotation: &'a InlineAnnotation,
    style: helix_view::graphics::Style,
    ready: bool,
}

impl<'a> BlameDecoration<'a> {
    pub fn new(annotation: &'a InlineAnnotation, theme: &helix_view::Theme) -> Self {
        Self {
            annotation,
            style: theme
                .try_get("ui.virtual.blame")
                .unwrap_or_else(|| theme.get("ui.virtual.inlay-hint")),
            ready: false,
        }
    }
}

impl super::text_decorations::Decoration for BlameDecoration<'_> {
    fn reset_pos(&mut self, pos: usize) -> usize {
        self.ready = false;
        if pos <= self.annotation.char_idx {
            self.annotation.char_idx
        } else {
            usize::MAX
        }
    }

    fn decorate_grapheme(
        &mut self,
        _: &mut super::document::TextRenderer,
        _: &helix_core::doc_formatter::FormattedGrapheme,
    ) -> usize {
        self.ready = true;
        usize::MAX
    }

    fn render_virt_lines(
        &mut self,
        renderer: &mut super::document::TextRenderer,
        pos: super::document::LinePos,
        offset: helix_core::Position,
    ) -> helix_core::Position {
        if self.ready {
            self.ready = false;
            let column = offset.col.saturating_sub(renderer.offset.col);
            if column < renderer.viewport.width as usize {
                renderer.set_stringn(
                    renderer.viewport.x + column as u16,
                    pos.visual_line,
                    &self.annotation.text,
                    renderer.viewport.width as usize - column,
                    self.style,
                );
            }
        }
        helix_core::Position::new(0, 0)
    }
}

pub(super) struct HunkDecoration<'a> {
    preview: &'a helix_view::annotations::git::InlineHunk,
    theme: &'a helix_view::Theme,
    tab_width: u16,
    ready: bool,
}

impl<'a> HunkDecoration<'a> {
    pub fn new(
        preview: &'a helix_view::annotations::git::InlineHunk,
        theme: &'a helix_view::Theme,
        tab_width: usize,
    ) -> Self {
        Self {
            preview,
            theme,
            tab_width: tab_width as u16,
            ready: false,
        }
    }
}

impl super::text_decorations::Decoration for HunkDecoration<'_> {
    fn reset_pos(&mut self, pos: usize) -> usize {
        self.ready = false;
        if pos <= self.preview.anchor {
            self.preview.anchor
        } else {
            usize::MAX
        }
    }

    fn decorate_grapheme(
        &mut self,
        _: &mut super::document::TextRenderer,
        _: &helix_core::doc_formatter::FormattedGrapheme,
    ) -> usize {
        self.ready = true;
        usize::MAX
    }

    fn render_virt_lines(
        &mut self,
        renderer: &mut super::document::TextRenderer,
        pos: super::document::LinePos,
        offset: helix_core::Position,
    ) -> helix_core::Position {
        use helix_core::{
            graphemes::Grapheme, unicode::segmentation::UnicodeSegmentation, Position,
        };
        use helix_view::{annotations::git::HunkLineKind, graphics::Rect};
        if !std::mem::take(&mut self.ready) {
            return Position::new(0, 0);
        }
        let start = pos.visual_line as usize + offset.row;
        let first = renderer.offset.row.saturating_sub(start);
        let end =
            self.preview.lines.len().min(
                (renderer.offset.row + renderer.viewport.height as usize).saturating_sub(start),
            );
        for index in first..end {
            let line = &self.preview.lines[index];
            let scope = match line.kind {
                HunkLineKind::Header => "diff.delta",
                HunkLineKind::Removed => "diff.minus",
                HunkLineKind::Added => "diff.plus",
            };
            let style = self.theme.get("ui.text").patch(self.theme.get(scope));
            let row = (start + index) as u16;
            renderer.set_style(
                Rect::new(renderer.viewport.x, row, renderer.viewport.width, 1),
                style,
            );
            let mut column = 0;
            for text in line.text.graphemes(true) {
                let grapheme = Grapheme::new(text.into(), column, self.tab_width);
                let control = !matches!(text, "\t") && text.chars().any(char::is_control);
                let width = if control { 1 } else { grapheme.width() };
                let end_column = column + width;
                let left = renderer.offset.col;
                let right = left + renderer.viewport.width as usize;
                if column >= right {
                    break;
                }
                if column >= left && end_column <= right {
                    let text = if text == "\t" {
                        " ".repeat(width)
                    } else if control {
                        "�".into()
                    } else {
                        text.into()
                    };
                    renderer.set_stringn(
                        renderer.viewport.x + (column - left) as u16,
                        row,
                        &text,
                        width,
                        style,
                    );
                }
                column = end_column;
            }
        }
        Position::new(self.preview.lines.len(), 0)
    }
}

pub(crate) fn show_hunk(cx: &mut commands::Context) {
    let (view, doc) = helix_view::current!(cx.editor);
    let line = doc
        .selection(view.id)
        .primary()
        .cursor_line(doc.text().slice(..));
    if let Some(preview) = view.inline_hunk(doc) {
        let range = &preview.hunk.after;
        if range.contains(&(line as u32)) || range.is_empty() && range.start as usize == line {
            view.inline_hunk = None;
            return;
        }
    }
    let Some(handle) = doc.diff_handle() else {
        cx.editor
            .set_status("Diff is not available in current buffer");
        return;
    };
    let diff = handle.load();
    // Diff workers can still be processing the latest edit. Never pair old ranges with new text.
    if diff.doc() != doc.text() {
        drop(diff);
        cx.editor.set_status("Diff is updating; try again");
        return;
    }
    let Some(index) = diff.hunk_at(line as u32, true) else {
        drop(diff);
        if view.inline_hunk.take().is_none() {
            cx.editor.set_status("No change at the cursor");
        }
        return;
    };
    view.inline_hunk = Some(helix_view::annotations::git::InlineHunk::new(
        doc,
        diff.diff_base().clone(),
        diff.nth_hunk(index),
    ));
}
