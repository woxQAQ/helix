use crate::compositor::{Component, Context};
use arc_swap::ArcSwap;
use tui::{
    buffer::Buffer as Surface,
    text::{Span, Spans, Text},
};

use std::sync::Arc;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use helix_core::{
    syntax::{self, HighlightEvent, OverlayHighlights},
    RopeSlice, Syntax,
};
use helix_view::{
    graphics::{Margin, Rect, Style},
    theme::Modifier,
    Theme,
};

fn styled_multiline_text<'a>(text: &str, style: Style) -> Text<'a> {
    let spans: Vec<_> = text
        .lines()
        .map(|line| Span::styled(line.to_string(), style))
        .map(Spans::from)
        .collect();
    Text::from(spans)
}

pub fn highlighted_code_block<'a>(
    text: &str,
    language: &str,
    theme: Option<&Theme>,
    loader: &syntax::Loader,
    // Optional overlay highlights to mix in with the syntax highlights.
    //
    // Note that `OverlayHighlights` is typically used with char indexing but the only caller
    // which passes this parameter currently passes **byte indices** instead.
    additional_highlight_spans: Option<OverlayHighlights>,
) -> Text<'a> {
    let mut spans = Vec::new();
    let mut lines = Vec::new();

    let get_theme = |key: &str| -> Style { theme.map(|t| t.get(key)).unwrap_or_default() };
    let text_style = get_theme(Markdown::TEXT_STYLE);
    let code_style = get_theme(Markdown::BLOCK_STYLE);

    let theme = match theme {
        Some(t) => t,
        None => return styled_multiline_text(text, code_style),
    };

    let ropeslice = RopeSlice::from(text);
    let Some(syntax) = loader
        .language_for_match(RopeSlice::from(language))
        .and_then(|lang| Syntax::new(ropeslice, lang, loader).ok())
    else {
        return styled_multiline_text(text, code_style);
    };

    let mut syntax_highlighter = syntax.highlighter(ropeslice, loader, ..);
    let mut syntax_highlight_stack = Vec::new();
    let mut overlay_highlight_stack = Vec::new();
    let mut overlay_highlighter = syntax::OverlayHighlighter::new(additional_highlight_spans);
    let mut pos = 0;

    while pos < ropeslice.len_bytes() as u32 {
        if pos == syntax_highlighter.next_event_offset() {
            let (event, new_highlights) = syntax_highlighter.advance();
            if event == HighlightEvent::Refresh {
                syntax_highlight_stack.clear();
            }
            syntax_highlight_stack.extend(new_highlights);
        } else if pos == overlay_highlighter.next_event_offset() as u32 {
            let (event, new_highlights) = overlay_highlighter.advance();
            if event == HighlightEvent::Refresh {
                overlay_highlight_stack.clear();
            }
            overlay_highlight_stack.extend(new_highlights)
        }

        let start = pos;
        pos = syntax_highlighter
            .next_event_offset()
            .min(overlay_highlighter.next_event_offset() as u32);
        if pos == u32::MAX {
            pos = ropeslice.len_bytes() as u32;
        }
        if pos == start {
            continue;
        }
        // The highlighter should always move forward.
        // If the highlighter malfunctions, bail on syntax highlighting and log an error.
        debug_assert!(pos > start);
        if pos < start {
            log::error!("Failed to highlight '{language}': {text:?}");
            return styled_multiline_text(text, code_style);
        }

        let style = syntax_highlight_stack
            .iter()
            .chain(overlay_highlight_stack.iter())
            .fold(text_style, |acc, highlight| {
                acc.patch(theme.highlight(*highlight))
            });

        let mut slice = &text[start as usize..pos as usize];
        // TODO: do we need to handle all unicode line endings
        // here, or is just '\n' okay?
        while let Some(end) = slice.find('\n') {
            // emit span up to newline
            let text = &slice[..end];
            let text = text.replace('\t', "    "); // replace tabs
            let span = Span::styled(text, style);
            spans.push(span);

            // truncate slice to after newline
            slice = &slice[end + 1..];

            // make a new line
            let spans = std::mem::take(&mut spans);
            lines.push(Spans::from(spans));
        }

        if !slice.is_empty() {
            let span = Span::styled(slice.replace('\t', "    "), style);
            spans.push(span);
        }
    }

    if !spans.is_empty() {
        let spans = std::mem::take(&mut spans);
        lines.push(Spans::from(spans));
    }

    Text::from(lines)
}

/// Markdown rendered into terminal text, together with where its source lines land.
///
/// `source_lines` holds `(source line, rendered line)` anchors ordered ascending in both
/// components. A pane that follows the buffer can look up a rendered line for the cursor's
/// source line instead of rendering the document once per line.
pub struct RenderedMarkdown {
    pub text: Text<'static>,
    source_lines: Vec<(usize, usize)>,
}

impl RenderedMarkdown {
    /// Rendered line that shows the block containing `source_line`.
    pub fn rendered_line_of(&self, source_line: usize) -> usize {
        match self
            .source_lines
            .binary_search_by_key(&source_line, |&(source, _)| source)
        {
            Ok(index) => self.source_lines[index].1,
            Err(0) => 0,
            Err(index) => self.source_lines[index - 1].1,
        }
    }
}

/// Records where a source line starts in the rendered text.
///
/// Only the first event of a source line is kept: the mapping stays ascending in both components,
/// which `RenderedMarkdown::rendered_line_of` relies on for its binary search. Events whose range
/// points backwards are ignored for the same reason.
fn anchor(source_lines: &mut Vec<(usize, usize)>, source_line: usize, rendered_line: usize) {
    match source_lines.last() {
        Some(&(last_source, last_rendered))
            if last_source >= source_line || last_rendered > rendered_line => {}
        _ => source_lines.push((source_line, rendered_line)),
    }
}

pub struct Markdown {
    contents: String,

    config_loader: Arc<ArcSwap<syntax::Loader>>,
}

impl Markdown {
    const TEXT_STYLE: &'static str = "ui.text";
    const BLOCK_STYLE: &'static str = "markup.raw.inline";
    const RULE_STYLE: &'static str = "punctuation.special";
    const UNNUMBERED_LIST_STYLE: &'static str = "markup.list.unnumbered";
    const NUMBERED_LIST_STYLE: &'static str = "markup.list.numbered";
    const HEADING_STYLES: [&'static str; 6] = [
        "markup.heading.1",
        "markup.heading.2",
        "markup.heading.3",
        "markup.heading.4",
        "markup.heading.5",
        "markup.heading.6",
    ];
    const INDENT: &'static str = "  ";

    pub fn new(contents: String, config_loader: Arc<ArcSwap<syntax::Loader>>) -> Self {
        Self {
            contents,
            config_loader,
        }
    }

    pub fn parse(&self, theme: Option<&Theme>) -> tui::text::Text<'static> {
        self.render_document(theme).text
    }

    /// Renders the document, also recording where its source lines end up.
    ///
    /// Callers that only draw the text, like popups, want [`Markdown::parse`]. A preview pane that
    /// follows the buffer needs the source line mapping to scroll along with the cursor.
    pub fn render_document(&self, theme: Option<&Theme>) -> RenderedMarkdown {
        fn push_line<'a>(spans: &mut Vec<Span<'a>>, lines: &mut Vec<Spans<'a>>) {
            let spans = std::mem::take(spans);
            if !spans.is_empty() {
                lines.push(Spans::from(spans));
            }
        }

        let mut options = Options::empty();
        options.insert(Options::ENABLE_STRIKETHROUGH);
        let parser = Parser::new_ext(&self.contents, options);

        // Start offset of every source line, used to translate the byte ranges reported by the
        // parser into buffer lines.
        let mut line_starts = vec![0];
        line_starts.extend(
            self.contents
                .match_indices('\n')
                .map(|(index, _)| index + 1),
        );
        let source_line = |offset: usize| {
            line_starts
                .partition_point(|&start| start <= offset)
                .saturating_sub(1)
        };

        // TODO: if possible, render links as terminal hyperlinks: https://gist.github.com/egmontkob/eb114294efbcd5adb1944c9f3cb5feda
        let mut tags = Vec::new();
        let mut spans = Vec::new();
        let mut lines = Vec::new();
        let mut list_stack = Vec::new();
        let mut source_lines = Vec::new();

        let get_indent = |level: usize| {
            if level < 1 {
                String::new()
            } else {
                Self::INDENT.repeat(level - 1)
            }
        };

        let get_theme = |key: &str| -> Style { theme.map(|t| t.get(key)).unwrap_or_default() };
        let text_style = get_theme(Self::TEXT_STYLE);
        let code_style = get_theme(Self::BLOCK_STYLE);
        let numbered_list_style = get_theme(Self::NUMBERED_LIST_STYLE);
        let unnumbered_list_style = get_theme(Self::UNNUMBERED_LIST_STYLE);
        let rule_style = get_theme(Self::RULE_STYLE);
        let heading_styles: Vec<Style> = Self::HEADING_STYLES
            .iter()
            .map(|key| get_theme(key))
            .collect();

        // Transform text in `<code>` blocks into `Event::Code`
        let mut in_code = false;
        let parser = parser
            .into_offset_iter()
            .filter_map(|(event, range)| match event {
                Event::Html(tag)
                    if tag.starts_with("<code")
                        && matches!(tag.chars().nth(5), Some(' ' | '>')) =>
                {
                    in_code = true;
                    None
                }
                Event::Html(tag) if *tag == *"</code>" => {
                    in_code = false;
                    None
                }
                Event::Text(text) if in_code => Some((Event::Code(text), range)),
                _ => Some((event, range)),
            });

        for (event, range) in parser {
            anchor(&mut source_lines, source_line(range.start), lines.len());
            match event {
                Event::Start(Tag::List(list)) => {
                    // if the list stack is not empty this is a sub list, in that
                    // case we need to push the current line before proceeding
                    if !list_stack.is_empty() {
                        push_line(&mut spans, &mut lines);
                    }

                    list_stack.push(list);
                }
                Event::End(TagEnd::List(_)) => {
                    list_stack.pop();

                    // whenever top-level list closes, empty line
                    if list_stack.is_empty() {
                        lines.push(Spans::default());
                    }
                }
                Event::Start(Tag::Item) => {
                    if list_stack.is_empty() {
                        log::warn!("markdown parsing error, list item without list");
                    }

                    tags.push(Tag::Item);

                    // get the appropriate bullet for the current list
                    let (bullet, bullet_style) = list_stack
                        .last()
                        .unwrap_or(&None) // use the '- ' bullet in case the list stack would be empty
                        .map_or((String::from("• "), unnumbered_list_style), |number| {
                            (format!("{}. ", number), numbered_list_style)
                        });

                    // increment the current list number if there is one
                    if let Some(v) = list_stack.last_mut().unwrap_or(&mut None).as_mut() {
                        *v += 1;
                    }

                    let prefix = get_indent(list_stack.len()) + bullet.as_str();
                    spans.push(Span::styled(prefix, bullet_style));
                }
                Event::Start(tag) => {
                    tags.push(tag);
                    if spans.is_empty() && !list_stack.is_empty() {
                        // TODO: could push indent + 2 or 3 spaces to align with
                        // the rest of the list.
                        spans.push(Span::from(get_indent(list_stack.len())));
                    }
                }
                Event::End(tag) => {
                    tags.pop();
                    match tag {
                        TagEnd::Heading(_)
                        | TagEnd::Paragraph
                        | TagEnd::CodeBlock
                        | TagEnd::Item => {
                            push_line(&mut spans, &mut lines);
                        }
                        _ => (),
                    }

                    // whenever heading, code block or paragraph closes, empty line
                    match tag {
                        TagEnd::Heading(_) | TagEnd::Paragraph | TagEnd::CodeBlock => {
                            lines.push(Spans::default());
                        }
                        _ => (),
                    }
                }
                Event::Text(text) => {
                    if let Some(Tag::CodeBlock(kind)) = tags.last() {
                        let language = match kind {
                            CodeBlockKind::Fenced(language) => language,
                            CodeBlockKind::Indented => "",
                        };
                        let rendered_start = lines.len();
                        let tui_text = highlighted_code_block(
                            &text,
                            language,
                            theme,
                            &self.config_loader.load(),
                            None,
                        );
                        lines.extend(tui_text.lines);
                        // Code blocks are not wrapped, so every source line of the block renders
                        // as exactly one line and can be anchored precisely.
                        let rendered_height = lines.len() - rendered_start;
                        let first_line = source_line(range.start);
                        for offset in 0..rendered_height {
                            anchor(
                                &mut source_lines,
                                first_line + offset,
                                rendered_start + offset,
                            );
                        }
                    } else {
                        let style = match tags.last() {
                            Some(Tag::Heading { level, .. }) => match level {
                                HeadingLevel::H1 => heading_styles[0],
                                HeadingLevel::H2 => heading_styles[1],
                                HeadingLevel::H3 => heading_styles[2],
                                HeadingLevel::H4 => heading_styles[3],
                                HeadingLevel::H5 => heading_styles[4],
                                HeadingLevel::H6 => heading_styles[5],
                            },
                            Some(Tag::Emphasis) => text_style.add_modifier(Modifier::ITALIC),
                            Some(Tag::Strong) => text_style.add_modifier(Modifier::BOLD),
                            Some(Tag::Strikethrough) => {
                                text_style.add_modifier(Modifier::CROSSED_OUT)
                            }
                            _ => text_style,
                        };
                        spans.push(Span::styled(text.into_string(), style));
                    }
                }
                Event::Code(text) | Event::Html(text) => {
                    spans.push(Span::styled(text.into_string(), code_style));
                }
                Event::SoftBreak | Event::HardBreak => {
                    push_line(&mut spans, &mut lines);
                    if !list_stack.is_empty() {
                        // TODO: could push indent + 2 or 3 spaces to align with
                        // the rest of the list.
                        spans.push(Span::from(get_indent(list_stack.len())));
                    }
                }
                Event::Rule => {
                    lines.push(Spans::from(Span::styled("───", rule_style)));
                    lines.push(Spans::default());
                }
                // TaskListMarker(bool) true if checked
                _ => {
                    log::warn!("unhandled markdown event {:?}", event);
                }
            }
            // build up a vec of Paragraph tui widgets
        }

        if !spans.is_empty() {
            lines.push(Spans::from(spans));
        }

        // if last line is empty, remove it
        if let Some(line) = lines.last() {
            if line.0.is_empty() {
                lines.pop();
            }
        }

        RenderedMarkdown {
            text: Text::from(lines),
            source_lines,
        }
    }
}

impl Component for Markdown {
    fn render(&mut self, area: Rect, surface: &mut Surface, cx: &mut Context) {
        use tui::widgets::{Paragraph, Widget, Wrap};

        let rendered = self.render_document(Some(&cx.editor.theme));

        let par = Paragraph::new(&rendered.text)
            .wrap(Wrap { trim: false })
            .scroll((cx.scroll.unwrap_or_default() as u16, 0));

        let margin = Margin::all(1);
        par.render(area.inner(margin), surface);
    }

    fn required_size(&mut self, viewport: (u16, u16)) -> Option<(u16, u16)> {
        let padding = 2;
        let rendered = self.render_document(None);

        // TODO: account for tab width
        let max_text_width = (viewport.0.saturating_sub(padding)).min(120);
        let (width, height) = crate::ui::text::required_size(&rendered.text, max_text_width);

        Some((width + padding, height + padding))
    }
}
