//! Interactive PTYs hosted in editor buffers or floating layers.
use std::{
    io::{Read, Write},
    path::Path,
    sync::Arc,
};

use anyhow::Context as _;
use helix_core::Position;
use helix_view::{
    graphics::{Color, CursorKind, Modifier, Rect, Style, UnderlineStyle},
    input::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    Editor,
};
use parking_lot::Mutex;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize};
use tokio::sync::mpsc;
use tui::{
    buffer::Buffer,
    widgets::{Block, Widget},
};

use crate::{
    compositor::{Component, Context, Event, EventResult},
    job,
};

pub fn open_lazygit(editor: &mut Editor, compositor: &mut crate::compositor::Compositor) {
    if compositor.find::<Terminal>().is_some() {
        return;
    }
    // The float paints on the editor background, so that is the terminal background
    // the child gets told about.
    let background = editor.theme.get("ui.background").bg;
    match Terminal::lazygit(
        &helix_loader::find_workspace().0,
        compositor.size(),
        background,
    ) {
        Ok(terminal) => compositor.push(Box::new(terminal)),
        Err(err) => editor.set_error(format!("{err:#}")),
    }
}

pub fn open_shell(
    directory: &Path,
    editor: &mut Editor,
    compositor: &mut crate::compositor::Compositor,
) {
    let area = compositor.size();
    if let Some(view) = compositor.find::<super::EditorView>() {
        if let Err(err) = view.open_terminal_panel(directory, area, editor) {
            editor.set_error(format!("{err:#}"));
        }
    }
}

pub fn is_toggle_key(key: &KeyEvent) -> bool {
    key.modifiers == KeyModifiers::CONTROL && matches!(key.code, KeyCode::Char('/' | '_'))
}

/// Longer than any query prefix we hold back for the rest of its bytes.
const MAX_QUERY: usize = 9;

/// Queries whose replies we generate: primary device attributes, the text area
/// size report and the color scheme report. The background color query is an OSC
/// sequence and is matched separately.
const QUERIES: [&[u8]; 3] = [b"\x1b[c", b"\x1b[18t", b"\x1b[?996n"];

/// Asks for the background color. Terminals are meant to answer with an OSC 11
/// report ending in BEL or ST.
const BACKGROUND_QUERY: &[u8] = b"\x1b]11;?";

/// Answers the terminal queries a full-screen child sends while it negotiates
/// its capabilities, size and colors.
///
/// tcell-based programs such as lazygit ask for the device attributes and wait up
/// to a second for the reply before drawing anything, so a terminal that stays
/// silent costs a second on every startup. We answer the subset that libvterm,
/// and therefore Neovim's `:terminal`, answers as well, plus the background color
/// so children can pick a light or dark theme of their own.
struct QueryReplies {
    /// The start of a query that continues in the next read.
    pending: Vec<u8>,
    /// The background color of the surface the terminal is drawn on.
    background: Option<(u8, u8, u8)>,
}

impl QueryReplies {
    fn new(background: Option<Color>) -> Self {
        Self {
            pending: Vec::new(),
            background: background.and_then(rgb),
        }
    }

    /// Scans a chunk of child output and returns the bytes to write back.
    fn feed(&mut self, chunk: &[u8], (rows, cols): (u16, u16)) -> Vec<u8> {
        let mut data = std::mem::take(&mut self.pending);
        data.extend_from_slice(chunk);
        let mut replies = Vec::new();
        let mut index = 0;
        while index < data.len() {
            let rest = &data[index..];
            if rest.starts_with(QUERIES[0]) {
                replies.extend_from_slice(b"\x1b[?1;2c");
                index += QUERIES[0].len();
            } else if rest.starts_with(QUERIES[1]) {
                replies.extend_from_slice(format!("\x1b[8;{rows};{cols}t").as_bytes());
                index += QUERIES[1].len();
            } else if rest.starts_with(QUERIES[2]) {
                // 1 is a dark and 2 a light background. lazygit derives both from the
                // background color it is told, so keep the two answers consistent.
                if let Some(background) = self.background {
                    let scheme = if is_light(background) { 2 } else { 1 };
                    replies.extend_from_slice(format!("\x1b[?997;{scheme}n").as_bytes());
                }
                index += QUERIES[2].len();
            } else if let Some(query) = rest.strip_prefix(BACKGROUND_QUERY) {
                // Wait for the terminator: a child may send it in a later write.
                let terminator = if query.starts_with(b"\x07") {
                    1
                } else if query.starts_with(b"\x1b\\") {
                    2
                } else if query.len() < 2 {
                    break;
                } else {
                    index += 1;
                    continue;
                };
                if let Some((r, g, b)) = self.background {
                    replies.extend_from_slice(
                        format!("\x1b]11;rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}\x07")
                            .as_bytes(),
                    );
                }
                index += BACKGROUND_QUERY.len() + terminator;
            } else if is_query_prefix(rest) {
                break;
            } else {
                index += 1;
            }
        }
        self.pending.extend_from_slice(&data[index..]);
        replies
    }
}

/// Whether `bytes` may be the beginning of a query whose remaining bytes have not
/// arrived yet.
fn is_query_prefix(bytes: &[u8]) -> bool {
    bytes.len() < MAX_QUERY
        && QUERIES
            .iter()
            .copied()
            .chain([BACKGROUND_QUERY])
            .any(|query| query.starts_with(bytes))
}

/// The sRGB value of a color, or `None` for the terminal's own default.
fn rgb(color: Color) -> Option<(u8, u8, u8)> {
    // The xterm defaults of the sixteen ANSI colors, then the 6x6x6 cube and the
    // grayscale ramp that make up the remaining indexed colors.
    const ANSI: [(u8, u8, u8); 16] = [
        (0x00, 0x00, 0x00),
        (0xcd, 0x00, 0x00),
        (0x00, 0xcd, 0x00),
        (0xcd, 0xcd, 0x00),
        (0x00, 0x00, 0xee),
        (0xcd, 0x00, 0xcd),
        (0x00, 0xcd, 0xcd),
        (0xe5, 0xe5, 0xe5),
        (0x7f, 0x7f, 0x7f),
        (0xff, 0x00, 0x00),
        (0x00, 0xff, 0x00),
        (0xff, 0xff, 0x00),
        (0x5c, 0x5c, 0xff),
        (0xff, 0x00, 0xff),
        (0x00, 0xff, 0xff),
        (0xff, 0xff, 0xff),
    ];
    Some(match color {
        Color::Reset => return None,
        Color::Black => ANSI[0],
        Color::Red => ANSI[1],
        Color::Green => ANSI[2],
        Color::Yellow => ANSI[3],
        Color::Blue => ANSI[4],
        Color::Magenta => ANSI[5],
        Color::Cyan => ANSI[6],
        Color::Gray => ANSI[7],
        Color::LightRed => ANSI[8],
        Color::LightGreen => ANSI[9],
        Color::LightYellow => ANSI[10],
        Color::LightBlue => ANSI[11],
        Color::LightMagenta => ANSI[12],
        Color::LightCyan => ANSI[13],
        Color::LightGray => ANSI[14],
        Color::White => ANSI[15],
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(index) => match index {
            0..=15 => ANSI[index as usize],
            16..=231 => {
                let level = |step: u8| if step == 0 { 0 } else { 55 + 40 * step };
                let index = index - 16;
                (level(index / 36), level(index % 36 / 6), level(index % 6))
            }
            _ => {
                let shade = 8 + 10 * (index - 232);
                (shade, shade, shade)
            }
        },
    })
}

/// Whether a background color reads as light.
///
/// lazygit classifies the background itself from CIE Lab lightness, which crosses
/// 0.5 at a relative luminance of ~0.184, so the same threshold keeps our two
/// answers in agreement.
fn is_light((r, g, b): (u8, u8, u8)) -> bool {
    let linear = |channel: u8| {
        let channel = f32::from(channel) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b) > 0.1838
}

/// Replies to cursor queries at the exact point they occur in the output stream.
struct CursorReplies {
    input: mpsc::UnboundedSender<Vec<u8>>,
}

impl vt100::Callbacks for CursorReplies {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        if i1.is_none() && i2.is_none() && params == [&[6][..]] && c == 'n' {
            let (row, col) = screen.cursor_position();
            // Screen coordinates are zero-based; terminal reports are one-based.
            self.input
                .send(format!("\x1b[{};{}R", row + 1, col + 1).into_bytes())
                .ok();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TerminalId(pub u64);

pub struct Terminal {
    panel: Option<TerminalId>,
    name: String,
    master: Box<dyn MasterPty + Send>,
    killer: Option<Box<dyn ChildKiller + Send + Sync>>,
    parser: Arc<Mutex<vt100::Parser<CursorReplies>>>,
    input: mpsc::UnboundedSender<Vec<u8>>,
    area: Rect,
}

impl Terminal {
    /// `background` is the color of the surface the terminal is drawn on, which is
    /// reported to the child as its terminal background.
    pub fn new(
        command: CommandBuilder,
        area: Rect,
        background: Option<Color>,
    ) -> anyhow::Result<Self> {
        Self::spawn(command, area, background, None)
    }

    pub(super) fn for_panel(
        directory: &Path,
        id: TerminalId,
        area: Rect,
        background: Option<Color>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            directory.is_dir(),
            "Not a directory: {}",
            directory.display()
        );
        let mut command = CommandBuilder::new_default_prog();
        command.cwd(directory);
        Self::spawn(command, area, background, Some(id))
    }

    fn spawn(
        mut command: CommandBuilder,
        area: Rect,
        background: Option<Color>,
        panel: Option<TerminalId>,
    ) -> anyhow::Result<Self> {
        let name = command
            .get_argv()
            .first()
            .and_then(|program| Path::new(program).file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "terminal".to_owned());
        let area = if panel.is_some() {
            area
        } else {
            Self::inner_area(area)
        };
        anyhow::ensure!(
            area.width > 0 && area.height > 0,
            "Terminal is too small for {name}"
        );
        command.env("TERM", "xterm-256color");
        // The child speaks the terminal protocol we implement, independently of the host.
        command.env_remove("TERM_PROGRAM");
        command.env_remove("TERM_PROGRAM_VERSION");
        let pair = portable_pty::native_pty_system().openpty(Self::pty_size(area))?;
        let mut reader = pair.master.try_clone_reader()?;
        let mut writer = pair.master.take_writer()?;
        let mut child = pair
            .slave
            .spawn_command(command)
            .with_context(|| format!("Failed to start {name} (is it installed and on PATH?)"))?;
        drop(pair.slave);
        let killer = child.clone_killer();
        let (output_tx, mut output_rx) = mpsc::channel::<Vec<u8>>(16);
        let (input, mut input_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let parser = Arc::new(Mutex::new(vt100::Parser::new_with_callbacks(
            area.height,
            area.width,
            0,
            CursorReplies {
                input: input.clone(),
            },
        )));
        tokio::task::spawn_blocking(move || {
            while let Some(bytes) = input_rx.blocking_recv() {
                if writer
                    .write_all(&bytes)
                    .and_then(|()| writer.flush())
                    .is_err()
                {
                    break;
                }
            }
        });
        let output = tokio::task::spawn_blocking(move || {
            let mut buffer = [0; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if output_tx.blocking_send(buffer[..count].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                    // Linux PTYs report EIO when the slave closes.
                    Err(_) => break,
                }
            }
            drop(output_tx);
            child.wait()
        });
        let screen = parser.clone();
        let replies = input.clone();
        let process_name = name.clone();
        tokio::spawn(async move {
            let mut queries = QueryReplies::new(background);
            while let Some(bytes) = output_rx.recv().await {
                let size = {
                    let mut screen = screen.lock();
                    screen.process(&bytes);
                    screen.screen().size()
                };
                let answer = queries.feed(&bytes, size);
                if !answer.is_empty() {
                    replies.send(answer).ok();
                }
                helix_event::request_redraw();
            }
            let result = output.await;
            job::dispatch(move |editor, compositor| {
                // A previous process must never close a newly opened terminal.
                let terminal = if let Some(id) = panel {
                    compositor
                        .find::<super::EditorView>()
                        .and_then(|view| view.terminal_mut(id))
                } else {
                    compositor.find::<Self>()
                };
                let Some(terminal) = terminal else {
                    return;
                };
                if !Arc::ptr_eq(&terminal.parser, &screen) {
                    return;
                }
                // The process has already been reaped; its PID may now be reused.
                terminal.killer = None;
                if let Some(id) = panel {
                    if let Some(view) = compositor.find::<super::EditorView>() {
                        view.remove_terminal(id);
                    }
                } else {
                    compositor.remove_type::<Self>();
                }
                match result {
                    Ok(Ok(status)) if status.success() => (),
                    Ok(Ok(status)) => editor.set_error(format!("{process_name} exited: {status}")),
                    Ok(Err(err)) => {
                        editor.set_error(format!("Failed to wait for {process_name}: {err}"))
                    }
                    Err(err) => editor.set_error(format!("Terminal task failed: {err}")),
                }
                helix_event::request_redraw();
            })
            .await;
        });
        Ok(Self {
            panel,
            name,
            master: pair.master,
            killer: Some(killer),
            parser,
            input,
            area,
        })
    }

    pub fn lazygit(
        directory: &Path,
        area: Rect,
        background: Option<Color>,
    ) -> anyhow::Result<Self> {
        let mut command = CommandBuilder::new("lazygit");
        command.cwd(directory);
        Self::new(command, area, background)
    }

    fn outer_area(area: Rect) -> Rect {
        let margin_x = area.width / 20;
        let margin_y = area.height / 20;
        Rect::new(
            area.x + margin_x,
            area.y + margin_y,
            area.width - 2 * margin_x,
            area.height - 2 * margin_y,
        )
    }

    fn inner_area(area: Rect) -> Rect {
        Block::bordered().inner(Self::outer_area(area))
    }

    fn pty_size(area: Rect) -> PtySize {
        PtySize {
            rows: area.height,
            cols: area.width,
            pixel_width: 0,
            pixel_height: 0,
        }
    }

    fn send(&self, bytes: Vec<u8>, cx: &mut Context) {
        if !bytes.is_empty() && self.input.send(bytes).is_err() {
            cx.editor
                .set_error(format!("The {} terminal has closed", self.name));
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        if let Some(killer) = &mut self.killer {
            let _ = killer.kill();
        }
    }
}

impl Component for Terminal {
    fn handle_event(&mut self, event: &Event, cx: &mut Context) -> EventResult {
        match event {
            Event::Key(key)
                if key.code == KeyCode::Char('g') && key.modifiers == KeyModifiers::CONTROL =>
            {
                let panel = self.panel;
                return EventResult::Consumed(Some(Box::new(move |compositor, _| {
                    if let Some(id) = panel {
                        if let Some(view) = compositor.find::<super::EditorView>() {
                            view.remove_terminal(id);
                        }
                    } else {
                        compositor.remove_type::<Self>();
                    }
                })));
            }
            Event::Key(key) => {
                let application_cursor = self.parser.lock().screen().application_cursor();
                self.send(encode_key(*key, application_cursor), cx);
            }
            Event::Paste(text) => {
                let bytes = if self.parser.lock().screen().bracketed_paste() {
                    format!("\x1b[200~{text}\x1b[201~").into_bytes()
                } else {
                    text.replace('\n', "\r").into_bytes()
                };
                self.send(bytes, cx);
            }
            Event::Mouse(mouse) => {
                let parser = self.parser.lock();
                let bytes = encode_mouse(mouse, self.area, parser.screen());
                drop(parser);
                self.send(bytes, cx);
            }
            Event::Resize(_, _) => return EventResult::Ignored(None),
            _ => (),
        }
        EventResult::Consumed(None)
    }

    fn render(&mut self, area: Rect, surface: &mut Buffer, cx: &mut Context) {
        self.render_in_area(area, surface, cx);
    }

    fn cursor(&self, _area: Rect, _editor: &Editor) -> (Option<Position>, CursorKind) {
        let parser = self.parser.lock();
        let screen = parser.screen();
        let (row, col) = screen.cursor_position();
        if screen.hide_cursor() || row >= self.area.height || col >= self.area.width {
            return (None, CursorKind::Hidden);
        }
        (
            Some(Position::new(
                (self.area.y + row).into(),
                (self.area.x + col).into(),
            )),
            CursorKind::Block,
        )
    }
}

impl Terminal {
    pub(super) fn resize(&mut self, area: Rect, editor: &mut Editor) {
        if area.width > 0
            && area.height > 0
            && (area.width != self.area.width || area.height != self.area.height)
        {
            if let Err(err) = self.master.resize(Self::pty_size(area)) {
                editor.set_error(format!("Failed to resize {}: {err}", self.name));
            }
            self.parser
                .lock()
                .screen_mut()
                .set_size(area.height, area.width);
        }
        self.area = area;
    }

    pub(super) fn render_in_area(&mut self, area: Rect, surface: &mut Buffer, cx: &mut Context) {
        let (outer, inner) = if self.panel.is_some() {
            (area, area)
        } else {
            (Self::outer_area(area), Self::inner_area(area))
        };
        self.resize(inner, cx.editor);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        for y in outer.top()..outer.bottom() {
            for x in outer.left()..outer.right() {
                surface[(x, y)].reset();
            }
        }
        if self.panel.is_none() {
            Block::bordered()
                .title(format!(" {} · Ctrl-g: close ", self.name))
                .style(
                    cx.editor
                        .theme
                        .get("ui.background")
                        .patch(cx.editor.theme.get("ui.text")),
                )
                .border_style(cx.editor.theme.get("ui.text.inactive"))
                .render(outer, surface);
        }
        let parser = self.parser.lock();
        let screen = parser.screen();
        let mut text = String::new();
        for row in 0..inner.height {
            let y = inner.y + row;
            let mut col = 0;
            while col < inner.width {
                let Some(cell) = screen.cell(row, col) else {
                    break;
                };
                if cell.is_wide_continuation() {
                    col += 1;
                    continue;
                }
                // Full-screen children such as lazygit repaint constantly, so a row is
                // drawn as runs of equally styled cells instead of one cell at a time.
                let style = cell_style(cell);
                text.clear();
                text.push_str(cell_text(cell));
                let mut end = col + 1;
                while end < inner.width {
                    let Some(cell) = screen.cell(row, end) else {
                        break;
                    };
                    if !cell.is_wide_continuation() {
                        if cell_style(cell) != style {
                            break;
                        }
                        text.push_str(cell_text(cell));
                    }
                    end += 1;
                }
                surface.set_stringn(inner.x + col, y, &text, (inner.width - col) as usize, style);
                col = end;
            }
        }
    }
}

fn color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(index) => Color::Indexed(index),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// The style a cell is drawn with.
fn cell_style(cell: &vt100::Cell) -> Style {
    let mut style = Style::default()
        .fg(color(cell.fgcolor()))
        .bg(color(cell.bgcolor()));
    for (enabled, modifier) in [
        (cell.bold(), Modifier::BOLD),
        (cell.dim(), Modifier::DIM),
        (cell.italic(), Modifier::ITALIC),
        (cell.inverse(), Modifier::REVERSED),
    ] {
        if enabled {
            style = style.add_modifier(modifier);
        }
    }
    if cell.underline() {
        style = style.underline_style(UnderlineStyle::Line);
    }
    style
}

/// The glyph a cell is drawn with; empty cells are blank.
fn cell_text(cell: &vt100::Cell) -> &str {
    if cell.has_contents() {
        cell.contents()
    } else {
        " "
    }
}

fn encode_key(key: KeyEvent, application_cursor: bool) -> Vec<u8> {
    let modifiers = key.modifiers;
    let parameter = 1
        + u8::from(modifiers.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(modifiers.contains(KeyModifiers::ALT))
        + 4 * u8::from(modifiers.contains(KeyModifiers::CONTROL));
    let mut bytes = Vec::new();
    match key.code {
        KeyCode::Char(ch) => {
            if modifiers.contains(KeyModifiers::ALT) {
                bytes.push(0x1b);
            }
            if modifiers.contains(KeyModifiers::CONTROL) {
                match ch {
                    '@' | ' ' | '2' => bytes.push(0),
                    'a'..='z' => bytes.push(ch as u8 - b'a' + 1),
                    'A'..='_' => bytes.push(ch as u8 - b'A' + 1),
                    '?' | '8' => bytes.push(0x7f),
                    _ => (),
                }
            } else {
                bytes.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
            }
        }
        KeyCode::Tab if modifiers.contains(KeyModifiers::SHIFT) => {
            bytes.extend_from_slice(b"\x1b[Z")
        }
        KeyCode::Enter | KeyCode::Tab | KeyCode::Backspace | KeyCode::Esc => {
            if modifiers.contains(KeyModifiers::ALT) {
                bytes.push(0x1b);
            }
            bytes.push(match key.code {
                KeyCode::Enter => b'\r',
                KeyCode::Tab => b'\t',
                KeyCode::Backspace => 0x7f,
                _ => 0x1b,
            });
        }
        KeyCode::Up
        | KeyCode::Down
        | KeyCode::Right
        | KeyCode::Left
        | KeyCode::Home
        | KeyCode::End => {
            let suffix = match key.code {
                KeyCode::Up => 'A',
                KeyCode::Down => 'B',
                KeyCode::Right => 'C',
                KeyCode::Left => 'D',
                KeyCode::Home => 'H',
                _ => 'F',
            };
            let sequence = if parameter != 1 {
                format!("\x1b[1;{parameter}{suffix}")
            } else if application_cursor {
                format!("\x1bO{suffix}")
            } else {
                format!("\x1b[{suffix}")
            };
            bytes.extend_from_slice(sequence.as_bytes());
        }
        KeyCode::Insert | KeyCode::Delete | KeyCode::PageUp | KeyCode::PageDown | KeyCode::F(_) => {
            let number = match key.code {
                KeyCode::Insert => 2,
                KeyCode::Delete => 3,
                KeyCode::PageUp => 5,
                KeyCode::PageDown => 6,
                KeyCode::F(n @ 1..=4) => {
                    let suffix = (b'P' + n - 1) as char;
                    return if parameter == 1 {
                        format!("\x1bO{suffix}")
                    } else {
                        format!("\x1b[1;{parameter}{suffix}")
                    }
                    .into_bytes();
                }
                KeyCode::F(5) => 15,
                KeyCode::F(6) => 17,
                KeyCode::F(7) => 18,
                KeyCode::F(8) => 19,
                KeyCode::F(9) => 20,
                KeyCode::F(10) => 21,
                KeyCode::F(11) => 23,
                KeyCode::F(12) => 24,
                _ => return bytes,
            };
            let sequence = if parameter == 1 {
                format!("\x1b[{number}~")
            } else {
                format!("\x1b[{number};{parameter}~")
            };
            bytes.extend_from_slice(sequence.as_bytes());
        }
        _ => (),
    }
    bytes
}

fn encode_mouse(mouse: &MouseEvent, area: Rect, screen: &vt100::Screen) -> Vec<u8> {
    use vt100::{MouseProtocolEncoding, MouseProtocolMode};
    if screen.mouse_protocol_mode() == MouseProtocolMode::None
        || mouse.column < area.x
        || mouse.column >= area.right()
        || mouse.row < area.y
        || mouse.row >= area.bottom()
    {
        return Vec::new();
    }
    let (mut button, released) = match mouse.kind {
        MouseEventKind::Down(button)
        | MouseEventKind::Up(button)
        | MouseEventKind::Drag(button) => {
            let code = match button {
                MouseButton::Left => 0,
                MouseButton::Middle => 1,
                MouseButton::Right => 2,
            };
            (code, matches!(mouse.kind, MouseEventKind::Up(_)))
        }
        MouseEventKind::ScrollUp => (64, false),
        MouseEventKind::ScrollDown => (65, false),
        MouseEventKind::ScrollLeft => (66, false),
        MouseEventKind::ScrollRight => (67, false),
        _ => return Vec::new(),
    };
    if matches!(mouse.kind, MouseEventKind::Drag(_)) {
        if !matches!(
            screen.mouse_protocol_mode(),
            MouseProtocolMode::ButtonMotion | MouseProtocolMode::AnyMotion
        ) {
            return Vec::new();
        }
        button += 32;
    }
    button += 4 * u16::from(mouse.modifiers.contains(KeyModifiers::SHIFT))
        + 8 * u16::from(mouse.modifiers.contains(KeyModifiers::ALT))
        + 16 * u16::from(mouse.modifiers.contains(KeyModifiers::CONTROL));
    let x = mouse.column - area.x + 1;
    let y = mouse.row - area.y + 1;
    if screen.mouse_protocol_encoding() == MouseProtocolEncoding::Sgr {
        format!("\x1b[<{button};{x};{y}{}", if released { 'm' } else { 'M' }).into_bytes()
    } else if x <= 223 && y <= 223 {
        vec![
            0x1b,
            b'[',
            b'M',
            if released { 35 } else { 32 + button as u8 },
            32 + x as u8,
            32 + y as u8,
        ]
    } else {
        Vec::new()
    }
}
