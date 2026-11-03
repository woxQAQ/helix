use helix_core::{
    doc_formatter::FormattedGrapheme, line_ending::line_without_line_ending,
    text_annotations::LineAnnotation, Position, Rope,
};
use helix_vcs::Hunk;
use std::sync::Arc;

use crate::{Document, DocumentId};

#[derive(Clone, Copy)]
pub enum HunkLineKind {
    Header,
    Removed,
    Added,
}

pub struct HunkLine {
    pub text: String,
    pub kind: HunkLineKind,
}

/// A preview belongs to one buffer snapshot; edits invalidate its ranges.
#[derive(Clone)]
pub struct InlineHunk {
    document: DocumentId,
    version: i32,
    base_revision: u64,
    pub hunk: Hunk,
    pub anchor: usize,
    pub lines: Arc<[HunkLine]>,
}

impl InlineHunk {
    pub fn new(doc: &Document, base: Rope, hunk: Hunk) -> Self {
        let mut lines = vec![HunkLine {
            text: format!(
                "@@ -{},{} +{},{} @@",
                hunk.before.start + u32::from(!hunk.before.is_empty()),
                hunk.before.len(),
                hunk.after.start + u32::from(!hunk.after.is_empty()),
                hunk.after.len(),
            ),
            kind: HunkLineKind::Header,
        }];
        for (prefix, text, range, kind) in [
            ('-', &base, hunk.before.clone(), HunkLineKind::Removed),
            ('+', doc.text(), hunk.after.clone(), HunkLineKind::Added),
        ] {
            for index in range {
                let line = text.line(index as usize);
                if line.len_chars() == 0 {
                    continue;
                }
                let contents = line_without_line_ending(&line, 0);
                lines.push(HunkLine {
                    text: format!("{prefix}{contents}"),
                    kind,
                });
                if line.len_chars() == contents.len_chars() {
                    lines.push(HunkLine {
                        text: "\\ No newline at end of file".into(),
                        kind: HunkLineKind::Header,
                    });
                }
            }
        }
        let line = hunk.after.end.saturating_sub(1).max(hunk.after.start) as usize;
        let line = line.min(doc.text().len_lines() - 1);
        let anchor = doc.text().line_to_char(line)
            + line_without_line_ending(&doc.text().slice(..), line).len_chars();
        Self {
            document: doc.id(),
            version: doc.version(),
            base_revision: doc.diff_base_revision(),
            hunk,
            anchor,
            lines: lines.into(),
        }
    }

    pub fn is_valid(&self, doc: &Document) -> bool {
        self.document == doc.id()
            && self.version == doc.version()
            && self.base_revision == doc.diff_base_revision()
            && doc.diff_handle().is_some()
    }

    pub fn line_annotation(&self) -> Box<dyn LineAnnotation> {
        Box::new(HunkAnnotation {
            anchor: self.anchor,
            height: self.lines.len(),
            ready: false,
        })
    }
}

struct HunkAnnotation {
    anchor: usize,
    height: usize,
    ready: bool,
}

impl LineAnnotation for HunkAnnotation {
    fn reset_pos(&mut self, char_idx: usize) -> usize {
        self.ready = false;
        if char_idx <= self.anchor {
            self.anchor
        } else {
            usize::MAX
        }
    }

    fn process_anchor(&mut self, _: &FormattedGrapheme) -> usize {
        self.ready = true;
        usize::MAX
    }

    fn insert_virtual_lines(&mut self, _: usize, _: Position, _: usize) -> Position {
        if std::mem::take(&mut self.ready) {
            Position::new(self.height, 0)
        } else {
            Position::new(0, 0)
        }
    }
}
