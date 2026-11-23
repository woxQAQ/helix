use std::{collections::BTreeMap, fmt::Write, path::PathBuf};

use helix_core::{
    diff::compare_ropes, doc_formatter::FormattedGrapheme, graphemes::grapheme_width,
    line_ending::line_end_char_index, text_annotations::LineAnnotation,
    unicode::segmentation::UnicodeSegmentation, ChangeSet, Position, Range, Rope,
};
use helix_stdx::path::get_relative_path;

use crate::{Document, Editor};

/// The comment identity and visual row within its frame in one view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReviewFocus {
    pub id: u64,
    pub row: usize,
}

pub struct ReviewRow<'a> {
    pub focus: ReviewFocus,
    pub text: &'a str,
    pub kind: ReviewRowKind,
    pub multiline_target: bool,
}

#[derive(Clone, Copy)]
pub enum ReviewRowKind {
    Header,
    Body,
    Footer,
}

#[derive(Clone, Copy)]
struct Anchor {
    range: Range,
    removed: bool,
}

impl Anchor {
    fn map(self, changes: &ChangeSet) -> Self {
        let range = self.range.map(changes);
        if !self.range.is_empty() && range.anchor >= range.head {
            Self {
                range: Range::point(range.head),
                removed: true,
            }
        } else {
            Self { range, ..self }
        }
    }
}

#[derive(Clone)]
pub(crate) struct Comment {
    id: u64,
    revision: usize,
    anchor: Anchor,
    text: String,
}

impl Comment {
    fn remove(comments: &mut Vec<Self>, id: u64) -> bool {
        let Some(index) = comments.iter().position(|comment| comment.id == id) else {
            return false;
        };
        comments.remove(index);
        true
    }
    fn resolve(&self, doc: &Document) -> Anchor {
        let mut anchor = self.anchor;
        if let Some(committed) = doc.committed_changes_since(self.revision) {
            anchor = anchor.map(committed.changes());
        }
        anchor.map(doc.changes())
    }
}

impl Editor {
    pub fn clear_review_comments(&mut self) -> anyhow::Result<()> {
        let review = self
            .review
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Start Helix with --comments to review files"))?;
        review.closed.clear();
        for doc in self.documents_mut() {
            doc.comments.clear();
            doc.review_notes.take();
        }
        Ok(())
    }
    pub fn add_review_comment(&mut self, text: String) -> anyhow::Result<()> {
        anyhow::ensure!(!text.trim().is_empty(), "Enter a comment");
        let comment = self.new_review_comment(text)?;
        let (_, doc) = crate::current!(self);
        doc.comments.push(comment);
        doc.review_notes.take();
        Ok(())
    }

    fn new_review_comment(&mut self, text: String) -> anyhow::Result<Comment> {
        let review = self
            .review
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Start Helix with --comments to review files"))?;
        let (view, doc) = crate::current!(self);
        anyhow::ensure!(
            doc.path().is_some(),
            "Save the buffer to a file before commenting"
        );
        doc.append_changes_to_history(view);
        let selection = doc.selection(view.id).primary();
        let (first, last) = selection.line_range(doc.text().slice(..));
        let range = if first == last {
            let start = doc.text().line_to_char(first);
            Range::new(start, start + doc.text().line(first).len_chars())
        } else {
            Range::new(selection.from(), selection.to())
        };
        doc.review_notes.take();
        let id = review.next_id;
        review.next_id += 1;
        Ok(Comment {
            id,
            revision: doc.current_revision(),
            anchor: Anchor {
                range,
                removed: false,
            },
            text,
        })
    }

    pub fn begin_review_edit(&mut self) -> anyhow::Result<()> {
        let (view, doc) = crate::current!(self);
        let comment = if let Some(focus) = doc.review_focus(view.id) {
            doc.comments
                .iter()
                .find(|comment| comment.id == focus.id)
                .unwrap()
                .clone()
        } else {
            self.new_review_comment(String::new())?
        };
        let (view, doc) = crate::current!(self);
        doc.review_draft = Some(ReviewDraft {
            view: view.id,
            cursor: comment.text.len(),
            comment,
        });
        doc.review_notes.take();
        Ok(())
    }
}

pub(crate) struct ReviewDraft {
    view: crate::ViewId,
    cursor: usize,
    comment: Comment,
}

impl Document {
    pub fn remove_review_comment(&mut self, id: u64) -> bool {
        let removed = Comment::remove(&mut self.comments, id);
        if removed {
            self.review_notes.take();
        }
        removed
    }
    pub fn review_text(&self, id: u64) -> Option<&str> {
        self.comments
            .iter()
            .find(|note| note.id == id)
            .map(|note| note.text.as_str())
    }

    pub fn review_input(&self) -> Option<&str> {
        Some(&self.review_draft.as_ref()?.comment.text)
    }

    pub fn update_review_input(&mut self, text: &str, cursor: usize) {
        if let Some(draft) = &mut self.review_draft {
            if draft.comment.text != text {
                draft.comment.text.clear();
                draft.comment.text.push_str(text);
                self.review_notes.take();
            }
            draft.cursor = cursor;
        }
    }

    pub fn finish_review_edit(&mut self, save: bool) {
        let Some(draft) = self.review_draft.take() else {
            return;
        };
        if save {
            if let Some(original) = self.comments.iter_mut().find(|c| c.id == draft.comment.id) {
                original.text = draft.comment.text;
            } else {
                self.comments.push(draft.comment);
            }
        }
        self.review_notes.take();
    }

    pub fn review_caret(&self, view: crate::ViewId, width: u16) -> Option<(ReviewFocus, usize)> {
        let draft = self
            .review_draft
            .as_ref()
            .filter(|draft| draft.view == view)?;
        if width == 0 {
            return None;
        }
        let (index, (start, _)) = body_rows(&draft.comment.text, width)
            .enumerate()
            .take_while(|(_, (start, _))| *start <= draft.cursor)
            .last()?;
        let col = draft.comment.text[start..draft.cursor]
            .graphemes(true)
            .map(grapheme_width)
            .sum::<usize>();
        Some((
            ReviewFocus {
                id: draft.comment.id,
                row: index + 1,
            },
            (2 + col).min(usize::from(width) - 1),
        ))
    }

    pub fn vertical_review_input_offset(
        &self,
        view: crate::ViewId,
        width: u16,
        down: bool,
    ) -> Option<usize> {
        let draft = self.review_draft.as_ref()?;
        let (focus, column) = self.review_caret(view, width)?;
        let row = if down {
            focus.row
        } else {
            focus.row.checked_sub(2)?
        };
        let (start, line) = body_rows(&draft.comment.text, width).nth(row)?;
        let mut col = 2;
        for (byte, grapheme) in line.grapheme_indices(true) {
            col += grapheme_width(grapheme);
            if col > column {
                return Some(start + byte);
            }
        }
        Some(start + line.len())
    }

    /// Share resolved anchors across layout, hit testing, and painting until the next edit.
    pub fn review_annotations(&self, width: u16) -> Option<ReviewAnnotations<'_>> {
        if (self.comments.is_empty() && self.review_draft.is_none()) || width == 0 {
            return None;
        }
        let notes = self.review_notes.get_or_init(|| {
            let text = self.text().slice(..);
            let draft = self.review_draft.as_ref().map(|d| &d.comment);
            let mut notes: Vec<_> = self
                .comments
                .iter()
                .filter(|c| draft.is_none_or(|d| d.id != c.id))
                .chain(draft)
                .map(|comment| {
                    let target = comment.resolve(self);
                    let (first, last) = target.range.line_range(text);
                    let mut label = format!("L{}", first + 1);
                    if first != last {
                        write!(label, "–L{}", last + 1).unwrap();
                    }
                    if target.removed {
                        label.push_str(" (target removed)");
                    }
                    let anchor = line_end_char_index(&text, last);
                    ReviewNote {
                        first,
                        last,
                        id: comment.id,
                        label,
                        anchor,
                        text: comment.text.chars().map(display_char).collect(),
                    }
                })
                .collect();
            notes.sort_by_key(|note| (note.anchor, note.id));
            notes
        });
        Some(ReviewAnnotations {
            notes,
            width,
            height: 0,
        })
    }
}

fn display_char(ch: char) -> char {
    match ch {
        '\t' => ' ',
        '\n' => '\n',
        ch if ch.is_control() => '�',
        ch => ch,
    }
}

pub(crate) struct ReviewNote {
    first: usize,
    last: usize,
    id: u64,
    label: String,
    anchor: usize,
    text: String,
}

pub struct ReviewAnnotations<'a> {
    notes: &'a [ReviewNote],
    width: u16,
    height: usize,
}

impl ReviewAnnotations<'_> {
    pub fn block_guide(&self, line: usize, first_visual_line: bool) -> Option<&'static str> {
        let mut start = false;
        for note in self
            .notes
            .iter()
            .filter(|n| n.first != n.last && n.first <= line && line <= n.last)
        {
            if note.first < line || !first_visual_line {
                return Some("│");
            }
            start = true;
        }
        start.then_some("╭")
    }

    pub fn rows_at(&self, anchor: usize) -> impl Iterator<Item = ReviewRow<'_>> {
        let start = self.notes.partition_point(|note| note.anchor < anchor);
        let width = self.width;
        self.notes[start..]
            .iter()
            .take_while(move |note| note.anchor == anchor)
            .flat_map(move |note| {
                let body =
                    body_rows(&note.text, width).map(|(_, text)| (ReviewRowKind::Body, text));
                std::iter::once((ReviewRowKind::Header, note.label.as_str()))
                    .chain(body)
                    .chain(std::iter::once((ReviewRowKind::Footer, "")))
                    .enumerate()
                    .map(move |(row, (kind, text))| ReviewRow {
                        focus: ReviewFocus { id: note.id, row },
                        kind,
                        multiline_target: note.first != note.last,
                        text,
                    })
            })
    }

    /// Resolve the focused row again after edits or a split resize.
    pub fn position(&self, focus: ReviewFocus) -> Option<(usize, usize)> {
        let anchor = self.notes.iter().find(|note| note.id == focus.id)?.anchor;
        let mut rows = self
            .rows_at(anchor)
            .enumerate()
            .filter(|(_, row)| row.focus.id == focus.id);
        let (first, _) = rows.next()?;
        Some((anchor, 1 + first + focus.row.min(rows.count())))
    }
}

fn body_rows(text: &str, width: u16) -> impl Iterator<Item = (usize, &str)> {
    let width = usize::from(width.saturating_sub(4).max(1));
    text.split('\n')
        .scan(0, |offset, line| {
            let start = *offset;
            *offset += line.len() + 1;
            Some((start, line))
        })
        .flat_map(move |(start, line)| {
            wrap_review_line(line, width).scan(start, |offset, chunk| {
                let start = *offset;
                *offset += chunk.len();
                Some((start, chunk))
            })
        })
}

/// Break only between grapheme clusters, including when a split is narrower than one cluster.
fn wrap_review_line(line: &str, width: usize) -> impl Iterator<Item = &str> {
    let mut rest = Some(line);
    std::iter::from_fn(move || {
        let line = rest.take()?;
        let mut col = 0;
        for (byte, grapheme) in line.grapheme_indices(true) {
            col += grapheme_width(grapheme);
            if byte != 0 && col > width {
                rest = Some(&line[byte..]);
                return Some(&line[..byte]);
            }
        }
        Some(line)
    })
}

impl LineAnnotation for ReviewAnnotations<'_> {
    fn reset_pos(&mut self, pos: usize) -> usize {
        self.height = 0;
        let index = self.notes.partition_point(|note| note.anchor < pos);
        self.notes.get(index).map_or(usize::MAX, |note| note.anchor)
    }

    fn process_anchor(&mut self, grapheme: &FormattedGrapheme) -> usize {
        let next = self.reset_pos(grapheme.char_idx + 1);
        self.height = self.rows_at(grapheme.char_idx).count();
        next
    }

    fn insert_virtual_lines(&mut self, _: usize, _: Position, _: usize) -> Position {
        Position::new(std::mem::take(&mut self.height), 0)
    }
}

/// Closed buffers retain one shared rope snapshot so their anchors can be rebased on reopen.
#[derive(Default)]
pub struct Review {
    next_id: u64,
    closed: BTreeMap<PathBuf, (Rope, Vec<Comment>)>,
}

pub struct Entry {
    pub path: PathBuf,
    id: u64,
    lines: (usize, usize),
    pub text: String,
    removed: bool,
}

impl Entry {
    pub fn location(&self) -> String {
        let mut location = get_relative_path(&self.path).display().to_string();
        let (first, last) = self.lines;
        write!(location, ":{}", first + 1).unwrap();
        if first != last {
            write!(location, "-{}", last + 1).unwrap();
        }
        if self.removed {
            location.push_str(" (target removed)");
        }
        location
    }

    pub fn range(&self, doc: &Document) -> Option<Range> {
        doc.comments
            .iter()
            .find(|comment| comment.id == self.id)
            .map(|comment| comment.resolve(doc).range)
    }

    pub fn remove(&self, editor: &mut Editor) -> bool {
        if let Some(doc) = editor.document_by_path_mut(&self.path) {
            if doc.remove_review_comment(self.id) {
                return true;
            }
        }
        let Some(review) = &mut editor.review else {
            return false;
        };
        let Some((_, comments)) = review.closed.get_mut(&self.path) else {
            return false;
        };
        let removed = Comment::remove(comments, self.id);
        if comments.is_empty() {
            review.closed.remove(&self.path);
        }
        removed
    }
}

impl Review {
    pub(crate) fn close(&mut self, doc: &mut Document) {
        if doc.comments.is_empty() {
            return;
        }
        let mut comments = std::mem::take(&mut doc.comments);
        for comment in &mut comments {
            comment.anchor = comment.resolve(doc);
        }
        if let Some(path) = doc.path() {
            self.closed
                .insert(path.to_owned(), (doc.text().clone(), comments));
        }
    }

    pub(crate) fn reopen(&mut self, doc: &mut Document) {
        let Some((text, mut comments)) = doc.path().and_then(|p| self.closed.remove(p)) else {
            return;
        };
        let edits = (text != *doc.text()).then(|| compare_ropes(&text, doc.text()));
        for comment in &mut comments {
            if let Some(edits) = &edits {
                comment.anchor = comment.anchor.map(edits.changes());
            }
            comment.revision = doc.current_revision();
        }
        doc.comments = comments;
    }

    pub fn entries(&self, editor: &Editor) -> Vec<Entry> {
        let mut entries = Vec::new();
        let mut collect =
            |path: &std::path::Path, text: &Rope, comment: &Comment, anchor: Anchor| {
                entries.push(Entry {
                    path: path.to_owned(),
                    id: comment.id,
                    lines: anchor.range.line_range(text.slice(..)),
                    removed: anchor.removed,
                    text: comment.text.clone(),
                });
            };
        for doc in editor.documents() {
            if let Some(path) = doc.path() {
                for comment in &doc.comments {
                    collect(path, doc.text(), comment, comment.resolve(doc));
                }
            }
        }
        for (path, (text, comments)) in &self.closed {
            for comment in comments {
                collect(path, text, comment, comment.anchor);
            }
        }
        entries.sort_by(|a, b| (&a.path, a.lines).cmp(&(&b.path, b.lines)));
        entries
    }

    pub fn report(&self, editor: &Editor) -> String {
        let mut output = String::new();
        for entry in self.entries(editor) {
            writeln!(output, "{} — {}", entry.location(), entry.text).unwrap();
        }
        output
    }
}
