use std::{
    collections::{HashMap, HashSet},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use arc_swap::ArcSwap;
use helix_vcs::FileChange;

use helix_view::{
    editor::Action,
    graphics::{Color, Rect, Style},
    input::{Event, MouseButton, MouseEventKind},
    Editor, Theme,
};
use tui::buffer::Buffer as Surface;

use crate::{ctrl, key};

use super::file_icons;

#[cfg(all(test, feature = "integration", feature = "git"))]
#[path = "file_explorer_tests.rs"]
mod tests;

struct Entry {
    path: PathBuf,
    name: String,
    icon: &'static str,
    is_dir: bool,
    depth: usize,
    expanded: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GitStatus {
    Untracked,
    Added,
    Deleted,
    Renamed,
    Modified,
    Conflict,
}

impl GitStatus {
    fn display(self) -> (&'static str, &'static str) {
        match self {
            Self::Untracked => ("?", "diff.plus"),
            Self::Added => ("A", "diff.plus"),
            Self::Deleted => ("D", "diff.minus"),
            Self::Renamed => ("R", "diff.delta.moved"),
            Self::Modified => ("M", "diff.delta"),
            Self::Conflict => ("!", "diff.delta.conflict"),
        }
    }

    fn foreground(self, theme: &Theme) -> Option<Color> {
        if self == Self::Modified {
            return theme
                .try_get_exact("ui.file-explorer.git.modified")
                .and_then(|style| style.fg)
                .or_else(|| theme.get("warning").fg)
                .or_else(|| theme.get("diff.delta").fg);
        }
        theme.get(self.display().1).fg
    }
}

/// A deduplicated set of Git states for a path and all of its descendants.
#[derive(Clone, Copy, Default)]
struct GitStatuses(u8);

impl GitStatuses {
    fn insert(&mut self, status: GitStatus) {
        self.0 |= 1 << status as u8;
    }

    fn iter(self) -> impl Iterator<Item = GitStatus> {
        // Keep badge order independent of Git's asynchronous traversal order.
        [
            GitStatus::Conflict,
            GitStatus::Modified,
            GitStatus::Renamed,
            GitStatus::Deleted,
            GitStatus::Added,
            GitStatus::Untracked,
        ]
        .into_iter()
        .filter(move |status| self.0 & (1 << *status as u8) != 0)
    }

    fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    fn label_width(self, width: usize) -> usize {
        width.saturating_sub(self.len() + usize::from(self.len() != 0))
    }

    fn render(self, right: u16, y: u16, surface: &mut Surface, theme: &Theme, base: Style) {
        let x = right - self.len() as u16;
        for (offset, status) in self.iter().enumerate() {
            let mut style = base;
            style.fg = status.foreground(theme).or(style.fg);
            surface.set_stringn(x + offset as u16, y, status.display().0, 1, style);
        }
    }
}

/// A persistent directory tree beside the editor's document views.
pub(super) struct FileExplorer {
    root: PathBuf,
    entries: Vec<Entry>,
    selected: usize,
    offset: usize,
    reveal_selection: bool,
    area: Rect,
    git_status: Arc<ArcSwap<HashMap<PathBuf, GitStatuses>>>,
    pub focused: bool,
}

pub(super) enum Outcome {
    Ignored,
    Consumed,
    Close,
    OpenExternal(PathBuf),
    Delete(PathBuf),
    Rename(PathBuf),
}

impl FileExplorer {
    pub fn new(root: PathBuf, editor: &Editor) -> io::Result<Self> {
        let entries = Self::children(&root, 0, editor)?;
        let mut explorer = Self {
            root,
            entries,
            selected: 0,
            offset: 0,
            reveal_selection: true,
            area: Rect::default(),
            git_status: Arc::new(ArcSwap::from_pointee(HashMap::new())),
            focused: true,
        };
        if let Some(path) = helix_view::doc!(editor).path() {
            // Reveal only the ancestors of the current file; other directories remain lazy.
            let mut index = 0;
            while index < explorer.entries.len() {
                let entry = &explorer.entries[index];
                if entry.path == path {
                    explorer.selected = index;
                    break;
                }
                if entry.is_dir && path.starts_with(&entry.path) {
                    explorer.expand(index, editor)?;
                }
                index += 1;
            }
        }
        explorer.refresh_git_status(editor);
        Ok(explorer)
    }

    fn refresh_git_status(&mut self, editor: &Editor) {
        // Each refresh publishes to its own snapshot, so an older scan cannot replace a newer one.
        let snapshot = Arc::new(ArcSwap::from(self.git_status.load_full()));
        self.git_status = snapshot.clone();
        let root = self.root.clone();
        let providers = editor.diff_providers.clone();
        let trust_full = editor
            .workspace_trust
            .query(
                &helix_loader::find_workspace_in(&root).0,
                helix_loader::workspace_trust::TrustQuery::Git,
            )
            .is_trusted();
        tokio::spawn(async move {
            let mut statuses = HashMap::<PathBuf, GitStatuses>::new();
            if let Some(changes) = providers.file_statuses(root.clone(), trust_full).await {
                for change in changes {
                    let status = match &change {
                        FileChange::Untracked { .. } => GitStatus::Untracked,
                        FileChange::Added { .. } => GitStatus::Added,
                        FileChange::Deleted { .. } => GitStatus::Deleted,
                        FileChange::Renamed { .. } => GitStatus::Renamed,
                        FileChange::Modified { .. } => GitStatus::Modified,
                        FileChange::Conflict { .. } => GitStatus::Conflict,
                    };
                    let mut record = |path: &Path| {
                        for ancestor in path.ancestors().take_while(|path| path.starts_with(&root))
                        {
                            statuses
                                .entry(ancestor.to_path_buf())
                                .or_default()
                                .insert(status);
                        }
                    };
                    record(change.path());
                    if let FileChange::Renamed { from_path, .. } = &change {
                        record(from_path);
                    }
                }
            }
            snapshot.store(Arc::new(statuses));
            helix_event::request_redraw();
        });
    }

    fn children(root: &Path, depth: usize, editor: &Editor) -> io::Result<Vec<Entry>> {
        root.read_dir()?;
        let loader = editor.syn_loader.load();
        Ok(super::directory_content(root, editor)?
            .into_iter()
            .filter(|(path, _)| *path != root.join(".."))
            .map(|(path, is_dir)| Entry {
                name: path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned(),
                icon: if is_dir {
                    file_icons::FOLDER
                } else {
                    file_icons::file_icon(&path, &loader)
                },
                path,
                is_dir,
                depth,
                expanded: false,
            })
            .collect())
    }

    fn expand(&mut self, index: usize, editor: &Editor) -> io::Result<()> {
        let entry = &self.entries[index];
        if !entry.is_dir || entry.expanded {
            return Ok(());
        }
        let children = Self::children(&entry.path, entry.depth + 1, editor)?;
        self.entries[index].expanded = true;
        self.entries.splice(index + 1..index + 1, children);
        Ok(())
    }

    fn collapse(&mut self) {
        let depth = self.entries[self.selected].depth;
        let end = self.entries[self.selected + 1..]
            .iter()
            .position(|entry| entry.depth <= depth)
            .map_or(self.entries.len(), |index| self.selected + 1 + index);
        self.entries.drain(self.selected + 1..end);
        self.entries[self.selected].expanded = false;
    }

    pub(super) fn root(&self) -> &Path {
        &self.root
    }

    pub(super) fn refresh(&mut self, editor: &Editor) -> io::Result<()> {
        let expanded: HashSet<_> = self
            .entries
            .iter()
            .filter(|entry| entry.expanded)
            .map(|entry| entry.path.clone())
            .collect();
        let selected = self.entries.get(self.selected).map(|entry| &entry.path);
        let mut refreshed = Self {
            root: self.root.clone(),
            entries: Self::children(&self.root, 0, editor)?,
            selected: self.selected,
            offset: self.offset,
            reveal_selection: self.reveal_selection,
            area: self.area,
            git_status: self.git_status.clone(),
            focused: self.focused,
        };
        let mut index = 0;
        while index < refreshed.entries.len() {
            if expanded.contains(&refreshed.entries[index].path) {
                refreshed.expand(index, editor)?;
            }
            if Some(&refreshed.entries[index].path) == selected {
                refreshed.selected = index;
            }
            index += 1;
        }
        refreshed.selected = refreshed
            .selected
            .min(refreshed.entries.len().saturating_sub(1));
        refreshed.refresh_git_status(editor);
        *self = refreshed;
        Ok(())
    }

    pub(super) fn rename_path(
        old_path: &Path,
        name: &str,
        editor: &mut Editor,
    ) -> io::Result<PathBuf> {
        let mut components = Path::new(name).components();
        if !matches!(components.next(), Some(std::path::Component::Normal(_)))
            || components.next().is_some()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "enter a name without directory components",
            ));
        }
        let new_path = old_path.with_file_name(name);
        if new_path == old_path {
            return Ok(new_path);
        }
        std::fs::symlink_metadata(old_path)?;
        match std::fs::symlink_metadata(&new_path) {
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} already exists", new_path.display()),
                ));
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        let descendants: Vec<_> = editor
            .documents()
            .filter_map(|doc| {
                let relative = doc.path()?.strip_prefix(old_path).ok()?;
                (!relative.as_os_str().is_empty()).then(|| (doc.id(), new_path.join(relative)))
            })
            .collect();
        editor.move_path(old_path, &new_path)?;
        for (id, path) in descendants {
            editor.set_doc_path(id, &path);
        }
        Ok(new_path)
    }

    pub(super) fn refresh_after_rename(
        &mut self,
        old_path: &Path,
        new_path: &Path,
        editor: &Editor,
    ) -> io::Result<()> {
        for entry in &mut self.entries {
            if let Ok(relative) = entry.path.strip_prefix(old_path) {
                entry.path = new_path.join(relative);
            }
        }
        self.refresh(editor)
    }

    fn activate(&mut self, editor: &mut Editor, action: Action) -> Outcome {
        let Some(entry) = self.entries.get(self.selected) else {
            return Outcome::Consumed;
        };
        let result = if entry.is_dir {
            if entry.expanded {
                self.collapse();
                Ok(())
            } else {
                self.expand(self.selected, editor)
                    .map_err(anyhow::Error::from)
            }
        } else {
            match crate::file_is_binary(&entry.path) {
                Ok(true) => return Outcome::OpenExternal(entry.path.clone()),
                Err(err) => {
                    editor.set_error(format!("File explorer: {err}"));
                    return Outcome::Consumed;
                }
                Ok(false) => {}
            }
            editor
                .open(&entry.path, action)
                .map(|_| self.focused = false)
                .map_err(Into::into)
        };
        if let Err(err) = result {
            editor.set_error(format!("File explorer: {err}"));
        }
        Outcome::Consumed
    }

    fn move_selection(&mut self, amount: isize) {
        self.selected = self
            .selected
            .saturating_add_signed(amount)
            .min(self.entries.len().saturating_sub(1));
    }

    pub fn handle_event(&mut self, event: &Event, editor: &mut Editor) -> Outcome {
        if self.focused && matches!(event, Event::Key(_) | Event::Paste(_)) {
            editor.status_msg = None;
            editor.reset_idle_timer();
            if matches!(event, Event::Key(_)) {
                self.reveal_selection = true;
            }
        }
        let result: anyhow::Result<()> = match event {
            Event::Mouse(mouse) => {
                if mouse.column < self.area.x
                    || mouse.column >= self.area.right()
                    || mouse.row < self.area.y
                    || mouse.row >= self.area.bottom()
                {
                    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                        self.focused = false;
                    }
                    return Outcome::Ignored;
                }
                match mouse.kind {
                    MouseEventKind::Down(_)
                    | MouseEventKind::ScrollUp
                    | MouseEventKind::ScrollDown => {
                        editor.status_msg = None;
                        editor.reset_idle_timer();
                    }
                    _ => {}
                }
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        self.focused = true;
                        if mouse.row > self.area.y && mouse.column < self.area.right() - 1 {
                            let index = self.offset + usize::from(mouse.row - self.area.y - 1);
                            if index < self.entries.len() {
                                self.selected = index;
                                self.reveal_selection = true;
                                return self.activate(editor, Action::Replace);
                            }
                        }
                    }
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                        let amount = if mouse.kind == MouseEventKind::ScrollUp {
                            -3
                        } else {
                            3
                        };
                        let height = usize::from(self.area.height.saturating_sub(1));
                        self.offset = self
                            .offset
                            .saturating_add_signed(amount)
                            .min(self.entries.len().saturating_sub(height));
                        self.reveal_selection = false;
                    }
                    _ => {}
                }
                return Outcome::Consumed;
            }
            Event::Key(key) if self.focused => match *key {
                key!(Esc) | key!(Tab) => {
                    self.focused = false;
                    return Outcome::Consumed;
                }
                key!('q') => return Outcome::Close,
                // Hand command prompts and window/keymap prefixes back to the editor.
                key!(':') | key!(' ') | ctrl!('w') => {
                    self.focused = false;
                    return Outcome::Ignored;
                }
                key!('j') | key!(Down) => {
                    self.move_selection(1);
                    Ok(())
                }
                key!('k') | key!(Up) => {
                    self.move_selection(-1);
                    Ok(())
                }
                key!(Home) | key!('g') => {
                    self.selected = 0;
                    Ok(())
                }
                key!(End) | key!('G') => {
                    self.selected = self.entries.len().saturating_sub(1);
                    Ok(())
                }
                key!(PageDown) | ctrl!('d') => {
                    self.move_selection(self.area.height.saturating_sub(2) as isize);
                    Ok(())
                }
                key!(PageUp) | ctrl!('u') => {
                    self.move_selection(-(self.area.height.saturating_sub(2) as isize));
                    Ok(())
                }
                key!('h') | key!(Left) => {
                    if let Some(entry) = self.entries.get(self.selected) {
                        if entry.expanded {
                            self.collapse();
                        } else if let Some(parent) = self.entries[..self.selected]
                            .iter()
                            .rposition(|parent| parent.depth < entry.depth)
                        {
                            self.selected = parent;
                        }
                    }
                    Ok(())
                }
                key!('l') | key!(Right) => {
                    if let Some(entry) = self.entries.get(self.selected) {
                        if entry.is_dir {
                            if entry.expanded {
                                if self
                                    .entries
                                    .get(self.selected + 1)
                                    .is_some_and(|child| child.depth > entry.depth)
                                {
                                    self.selected += 1;
                                }
                                Ok(())
                            } else {
                                self.expand(self.selected, editor).map_err(Into::into)
                            }
                        } else {
                            return self.activate(editor, Action::Replace);
                        }
                    } else {
                        Ok(())
                    }
                }
                key!(Enter) => return self.activate(editor, Action::Replace),
                ctrl!('v') => return self.activate(editor, Action::VerticalSplit),
                ctrl!('s') => return self.activate(editor, Action::HorizontalSplit),
                key!('d') => {
                    if let Some(entry) = self.entries.get(self.selected) {
                        if entry.is_dir {
                            editor.set_error("File explorer: select a file to delete");
                        } else {
                            return Outcome::Delete(entry.path.clone());
                        }
                    }
                    Ok(())
                }
                key!('r') => {
                    if let Some(entry) = self.entries.get(self.selected) {
                        return Outcome::Rename(entry.path.clone());
                    }
                    Ok(())
                }
                key!('R') => self.refresh(editor).map_err(Into::into),
                _ => Ok(()),
            },
            Event::Paste(_) if self.focused => return Outcome::Consumed,
            _ => return Outcome::Ignored,
        };
        if let Err(err) = result {
            editor.set_error(format!("File explorer: {err}"));
        }
        Outcome::Consumed
    }

    /// Reserve space only when both the tree and document remain usable.
    pub fn layout(&mut self, area: Rect, width: u16) -> Rect {
        let width = if area.width >= 33 && area.height >= 3 {
            width.clamp(12, area.width - 21)
        } else {
            self.focused = false;
            0
        };
        let explorer_area = area.clip_bottom(1).with_width(width);
        if explorer_area.height != self.area.height {
            self.reveal_selection = true;
        }
        self.area = explorer_area;
        Rect::new(area.x + width, area.y, area.width - width, area.height)
    }

    pub fn render(&mut self, surface: &mut Surface, editor: &Editor) {
        let area = self.area;
        if area.width == 0 {
            return;
        }
        let theme = &editor.theme;
        let icons = editor.config().file_explorer.icons;
        let git_status = self.git_status.load();
        let width = usize::from(area.width - 1);
        let header = if self.focused {
            "ui.statusline"
        } else {
            "ui.statusline.inactive"
        };
        surface.set_style(area.with_height(1), theme.get(header));
        let title = self
            .root
            .file_name()
            .unwrap_or(self.root.as_os_str())
            .to_string_lossy();
        let title = if icons {
            format!("{} {title}", file_icons::FOLDER_OPEN)
        } else {
            title.into_owned()
        };
        let root_status = git_status.get(&self.root).copied().unwrap_or_default();
        let header_style = theme.get(header);
        surface.set_stringn(
            area.x,
            area.y,
            &title,
            root_status.label_width(width),
            header_style,
        );
        root_status.render(area.right() - 1, area.y, surface, theme, header_style);

        let height = usize::from(area.height - 1);
        // Mouse scrolling may leave the selection outside the viewport.
        if self.reveal_selection {
            self.offset = self.offset.min(self.selected);
            if self.selected >= self.offset + height {
                self.offset = self.selected + 1 - height;
            }
            self.reveal_selection = false;
        }
        self.offset = self.offset.min(self.entries.len().saturating_sub(height));
        for (row, entry) in self
            .entries
            .iter()
            .skip(self.offset)
            .take(height)
            .enumerate()
        {
            let index = self.offset + row;
            let y = area.y + 1 + row as u16;
            let base = if index == self.selected {
                theme.get(if self.focused {
                    "ui.selection.primary"
                } else {
                    "ui.selection"
                })
            } else {
                theme.get("ui.background")
            };
            surface.set_style(Rect::new(area.x, y, area.width - 1, 1), base);
            let status = git_status.get(&entry.path).copied().unwrap_or_default();
            let label_width = status.label_width(width);
            let show_icon = icons && label_width >= 4;
            let icon = if show_icon {
                if entry.is_dir && entry.expanded {
                    file_icons::FOLDER_OPEN
                } else {
                    entry.icon
                }
            } else {
                ""
            };
            let icon_spacing = if show_icon { " " } else { "" };
            let prefix_width = if show_icon { 2 } else { 0 };
            let indent = (entry.depth * 2).min(label_width.saturating_sub(prefix_width + 1));
            let label = format!("{}{icon}{icon_spacing}{}", " ".repeat(indent), entry.name);
            let foreground = status
                .iter()
                .next()
                .and_then(|status| status.foreground(theme));
            let mut style = base.patch(theme.get(if entry.is_dir {
                "ui.text.directory"
            } else {
                "ui.text"
            }));
            // Preserve directory identity and selection backgrounds while coloring changed files.
            if !entry.is_dir {
                style.fg = foreground.or(style.fg);
            }
            surface.set_stringn(area.x, y, &label, label_width, style);
            status.render(area.right() - 1, y, surface, theme, style);
        }
        if self.entries.is_empty() && area.height > 1 {
            surface.set_stringn(
                area.x,
                area.y + 1,
                "(empty)",
                width,
                theme.get("ui.text.inactive"),
            );
        }
        for y in area.y..area.bottom() {
            surface.set_stringn(area.right() - 1, y, "│", 1, theme.get("ui.window"));
        }
        if self.entries.len() > height && height > 0 {
            let thumb_height = (height * height).div_ceil(self.entries.len());
            let thumb_top = (height - thumb_height) * self.offset / (self.entries.len() - height);
            let style = theme.get("ui.window").patch(theme.get("ui.menu.scroll"));
            for row in thumb_top..thumb_top + thumb_height {
                surface.set_stringn(area.right() - 1, area.y + 1 + row as u16, "▐", 1, style);
            }
        }
    }
}
