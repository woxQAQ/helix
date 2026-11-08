use super::*;

use std::{process::Command, time::Duration};

use crate::{application::Application, args::Args, config::Config};
use helix_loader::workspace_trust::WorkspaceTrust;
use helix_view::editor::{LspConfig, WordCompletion};

fn application() -> anyhow::Result<Application> {
    let mut config = Config::default();
    config.editor.lsp = LspConfig {
        enable: false,
        ..Default::default()
    };
    config.editor.word_completion = WordCompletion {
        enable: false,
        ..Default::default()
    };
    config.editor.file_explorer.hidden = true;
    config.editor.file_explorer.flatten_dirs = false;
    let syntax =
        helix_core::syntax::Loader::new(helix_loader::config::default_lang_config().try_into()?)?;
    Application::new(
        Args::default(),
        config,
        syntax,
        WorkspaceTrust::fully_trusted(),
    )
}

fn git(root: &Path, args: &[&str]) -> anyhow::Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=Helix Test",
            "-c",
            "user.email=test@helix.org",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn mouse_scrolling_preserves_selection_and_keyboard_reveals_it() -> anyhow::Result<()> {
    use helix_view::input::{KeyModifiers, MouseEvent};

    let directory = tempfile::tempdir()?;
    let root = directory.path().canonicalize()?;
    for index in 0..30 {
        std::fs::write(root.join(format!("file-{index:02}.txt")), "contents\n")?;
    }
    let mut editor = application()?.editor;
    let mut explorer = FileExplorer::new(root, &editor)?;
    let area = Rect::new(0, 0, 80, 12);
    explorer.layout(area, 30);
    let height = usize::from(explorer.area.height - 1);
    let mut surface = Surface::empty(area);
    explorer.render(&mut surface, &editor);
    assert_eq!(surface.get(29, 0).unwrap().symbol.as_str(), "│");
    assert_eq!(surface.get(29, 1).unwrap().symbol.as_str(), "▐");

    let scroll = |kind| {
        Event::Mouse(MouseEvent {
            kind,
            column: 3,
            row: 2,
            modifiers: KeyModifiers::empty(),
        })
    };
    explorer.handle_event(&scroll(MouseEventKind::ScrollDown), &mut editor);
    explorer.render(&mut surface, &editor);
    assert_eq!(explorer.selected, 0);
    assert_eq!(explorer.offset, 3);
    let first_row: String = (0..29)
        .map(|x| surface.get(x, 1).unwrap().symbol.as_str())
        .collect();
    assert!(first_row.contains("file-03.txt"));
    // Repeated redraws must not pull the viewport back to the selection.
    explorer.render(&mut surface, &editor);
    assert_eq!(explorer.offset, 3);
    for _ in 0..20 {
        explorer.handle_event(&scroll(MouseEventKind::ScrollDown), &mut editor);
    }
    explorer.render(&mut surface, &editor);
    assert_eq!(explorer.offset, explorer.entries.len() - height);
    assert_eq!(explorer.selected, 0);
    assert_eq!(
        surface
            .get(29, explorer.area.bottom() - 1)
            .unwrap()
            .symbol
            .as_str(),
        "▐"
    );

    explorer.handle_event(&Event::Key(key!('j')), &mut editor);
    explorer.render(&mut surface, &editor);
    assert_eq!(explorer.selected, 1);
    assert_eq!(explorer.offset, 1);
    explorer.handle_event(&Event::Key(key!(End)), &mut editor);
    explorer.render(&mut surface, &editor);
    assert_eq!(explorer.selected, 29);
    assert_eq!(explorer.offset, 20);
    for _ in 0..20 {
        explorer.handle_event(&scroll(MouseEventKind::ScrollUp), &mut editor);
    }
    explorer.render(&mut surface, &editor);
    assert_eq!(explorer.offset, 0);
    assert_eq!(explorer.selected, 29);

    // Clicking a row resolves the entry from the scrolled viewport.
    explorer.handle_event(&scroll(MouseEventKind::ScrollDown), &mut editor);
    explorer.render(&mut surface, &editor);
    let clicked = explorer.entries[4].path.clone();
    explorer.handle_event(
        &scroll(MouseEventKind::Down(MouseButton::Left)),
        &mut editor,
    );
    assert_eq!(explorer.selected, 4);
    assert_eq!(helix_view::doc!(editor).path(), Some(clicked.as_path()));

    // Resizing to fit all entries removes the scrollbar and clamps the offset.
    let large = Rect::new(0, 0, 80, 40);
    explorer.layout(large, 30);
    let mut surface = Surface::empty(large);
    explorer.render(&mut surface, &editor);
    assert_eq!(explorer.offset, 0);
    assert!((0..explorer.area.height).all(|y| surface.get(29, y).unwrap().symbol.as_str() == "│"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn binary_files_are_delegated_and_text_controls_stay_in_the_view() -> anyhow::Result<()> {
    use crate::{
        compositor::{Component, Context},
        job::Jobs,
        keymap::{self, Keymaps},
        ui::EditorView,
    };
    use helix_view::input::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

    let directory = tempfile::tempdir()?;
    let root = directory.path().canonicalize()?;
    let png = root.join("image.png");
    let disguised = root.join("binary.txt");
    let text = root.join("text.png");
    let utf16 = root.join("utf16.txt");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\n\0\x1b[2J\x08data")?;
    std::fs::write(&disguised, b"binary\0data")?;
    std::fs::write(&text, "plain text\n")?;
    std::fs::write(&utf16, b"\xff\xfeh\0i\0\n\0")?;
    let mut editor = application()?.editor;
    let original = helix_view::doc!(editor).id();
    let documents = editor.documents().count();
    let views = editor.tree.views().count();
    let area = Rect::new(0, 0, 100, 20);
    let mut explorer = FileExplorer::new(root.clone(), &editor)?;
    explorer.layout(area, 40);
    for path in [&png, &disguised] {
        explorer.selected = explorer
            .entries
            .iter()
            .position(|entry| &entry.path == path)
            .unwrap();
        let row = 1 + explorer.selected as u16;
        for event in [
            Event::Key(key!(Enter)),
            Event::Key(key!('l')),
            Event::Key(ctrl!('v')),
            Event::Key(ctrl!('s')),
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 4,
                row,
                modifiers: KeyModifiers::empty(),
            }),
        ] {
            // Inspect the external-open request without launching a desktop application.
            assert!(
                matches!(explorer.handle_event(&event, &mut editor), Outcome::OpenExternal(target) if &target == path)
            );
            assert_eq!(helix_view::doc!(editor).id(), original);
            assert_eq!(editor.documents().count(), documents);
            assert_eq!(editor.tree.views().count(), views);
            assert!(explorer.focused);
        }
    }
    for (path, expected) in [(&text, "plain text\n"), (&utf16, "hi\n")] {
        explorer.selected = explorer
            .entries
            .iter()
            .position(|entry| &entry.path == path)
            .unwrap();
        explorer.focused = true;
        assert!(matches!(
            explorer.handle_event(&Event::Key(key!(Enter)), &mut editor),
            Outcome::Consumed
        ));
        assert_eq!(helix_view::doc!(editor).text().to_string(), expected);
        assert!(!explorer.focused);
    }

    // Explicit text opens must also render escape/control bytes as printable placeholders.
    let controls = root.join("controls.txt");
    let content = "before\x1b[2J\x08\x00\x07\u{0085}\u{009b}after\n";
    std::fs::write(&controls, content)?;
    editor.open(&controls, Action::Replace)?;
    let mut view = EditorView::new(Keymaps::new(Box::new(ArcSwap::from_pointee(
        keymap::default(),
    ))));
    view.focus_file_explorer(&mut editor);
    let mut jobs = Jobs::new();
    let mut cx = Context {
        editor: &mut editor,
        scroll: None,
        jobs: &mut jobs,
    };
    let mut surface = Surface::empty(area);
    view.render(area, &mut surface, &mut cx);
    assert!(surface
        .content
        .iter()
        .all(|cell| !cell.symbol.chars().any(char::is_control)));
    assert!(surface
        .content
        .iter()
        .any(|cell| cell.symbol.as_str() == "�"));
    assert_eq!(helix_view::doc!(cx.editor).text().to_string(), content);
    assert_eq!(cx.editor.tree.views().next().unwrap().0.area.x, 30);
    Ok(())
}

/// Ignored entries remain navigable, inherit directory rules, and update on refresh.
#[tokio::test(flavor = "multi_thread")]
async fn ignored_entries_are_dimmed_and_refresh_with_rules() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().canonicalize()?;
    git(&root, &["init", "-q"])?;
    std::fs::create_dir_all(root.join("build/nested"))?;
    std::fs::create_dir(root.join("src"))?;
    std::fs::write(root.join(".gitignore"), "build/\n*.log\n!keep.log\n")?;
    std::fs::write(root.join("src/.gitignore"), "!debug.log\n")?;
    std::fs::write(root.join(".ignore"), "local.txt\n")?;
    std::fs::write(root.join(".git/info/exclude"), "excluded.txt\n")?;
    for name in [
        "build/nested/output.txt",
        "src/debug.log",
        "debug.log",
        "keep.log",
        "local.txt",
        "excluded.txt",
        "clean.txt",
    ] {
        std::fs::write(root.join(name), "contents\n")?;
    }
    let mut editor = application()?.editor;
    editor.theme =
        helix_view::theme::Loader::new(helix_loader::runtime_dirs()).load("catppuccin_mocha")?;
    let mut explorer = FileExplorer::new(root.clone(), &editor)?;
    let area = Rect::new(0, 0, 80, 30);
    explorer.layout(area, 40);
    // Use the normal navigation handler to enter an ignored directory.
    explorer.selected = explorer
        .entries
        .iter()
        .position(|entry| entry.name == "build")
        .unwrap();
    explorer.handle_event(&Event::Key(key!('l')), &mut editor);
    explorer.handle_event(&Event::Key(key!('j')), &mut editor);
    explorer.handle_event(&Event::Key(key!('l')), &mut editor);
    let src = explorer
        .entries
        .iter()
        .position(|entry| entry.name == "src")
        .unwrap();
    explorer.expand(src, &editor)?;

    for (name, ignored) in [
        ("build", true),
        ("nested", true),
        ("output.txt", true),
        ("src", false),
        ("debug.log", true),
        ("keep.log", false),
        ("local.txt", true),
        ("excluded.txt", true),
        ("clean.txt", false),
    ] {
        let indices: Vec<_> = explorer
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.name == name)
            .map(|(index, _)| index)
            .collect();
        assert!(!indices.is_empty(), "missing entry {name}");
        for index in indices {
            let ignored = ignored && explorer.entries[index].path != root.join("src/debug.log");
            explorer.selected = index;
            for focused in [true, false] {
                explorer.focused = focused;
                let mut surface = Surface::empty(area);
                explorer.render(&mut surface, &editor);
                let entry = &explorer.entries[index];
                assert_eq!(
                    entry.ignored,
                    ignored,
                    "ignore state for {}",
                    entry.path.display()
                );
                let cell = surface
                    .get((entry.depth * 2) as u16, index as u16 + 1)
                    .unwrap();
                if ignored {
                    assert_eq!(
                        cell.fg,
                        editor.theme.get("ui.text.inactive").fg.unwrap(),
                        "foreground for {name}"
                    );
                }
                let selection = if focused {
                    "ui.selection.primary"
                } else {
                    "ui.selection"
                };
                assert_eq!(cell.bg, editor.theme.get(selection).bg.unwrap());
            }
        }
    }
    // An explicit theme scope overrides the fallback without replacing the selection background.
    let original_theme = editor.theme.clone();
    let mut theme = helix_view::theme::DEFAULT_THEME_DATA.clone();
    theme.as_table_mut().unwrap().insert(
        "ui.file-explorer.ignored".into(),
        toml::from_str::<toml::Value>("fg = '#123456'\nbg = '#654321'\nmodifiers = ['italic']")?,
    );
    editor.theme = Theme::from(theme);
    explorer.selected = explorer
        .entries
        .iter()
        .position(|entry| entry.name == "local.txt")
        .unwrap();
    let mut surface = Surface::empty(area);
    explorer.render(&mut surface, &editor);
    let cell = surface.get(0, explorer.selected as u16 + 1).unwrap();
    assert_eq!(cell.fg, Color::Rgb(0x12, 0x34, 0x56));
    assert_eq!(cell.bg, editor.theme.get("ui.selection").bg.unwrap());
    assert!(cell
        .modifier
        .contains(helix_view::graphics::Modifier::ITALIC));
    editor.theme = original_theme;

    // Rules from outside an explorer's root still apply, including ignored ancestors.
    let nested = FileExplorer::new(root.join("build/nested"), &editor)?;
    assert!(
        nested
            .entries
            .iter()
            .find(|entry| entry.name == "output.txt")
            .unwrap()
            .ignored
    );

    explorer.focused = true;
    explorer.selected = explorer
        .entries
        .iter()
        .position(|entry| entry.name == "output.txt")
        .unwrap();
    explorer.handle_event(&Event::Key(key!(Enter)), &mut editor);
    assert_eq!(
        helix_view::doc!(editor).path(),
        Some(root.join("build/nested/output.txt").as_path())
    );

    std::fs::write(root.join(".gitignore"), "")?;
    explorer.focused = true;
    explorer.handle_event(&Event::Key(key!('R')), &mut editor);
    assert!(
        !explorer
            .entries
            .iter()
            .find(|entry| entry.name == "output.txt")
            .unwrap()
            .ignored
    );
    assert!(
        explorer
            .entries
            .iter()
            .find(|entry| entry.name == "local.txt")
            .unwrap()
            .ignored
    );

    // Picker flattening must not merge directories in the sidebar or hide their ignore status.
    std::fs::write(root.join(".gitignore"), "build/nested/\n")?;
    let mut config = (*editor.config()).clone();
    config.file_explorer.flatten_dirs = true;
    editor.config = Arc::new(ArcSwap::from_pointee(config));
    explorer.refresh(&editor)?;
    assert!(
        explorer
            .entries
            .iter()
            .find(|entry| entry.path == root.join("build/nested"))
            .unwrap()
            .ignored
    );
    Ok(())
}

/// Exercise the sidebar against a real repository, including index-only changes and refresh.
#[tokio::test(flavor = "multi_thread")]
async fn git_status_badges_and_directory_summaries() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().canonicalize()?;
    git(&root, &["init", "-q"])?;
    for name in ["changed", "removed", "moved"] {
        std::fs::create_dir(root.join(name))?;
    }
    for name in [
        "changed/modified.txt",
        "removed/deleted.txt",
        "removed/keep.txt",
        "moved/original.txt",
        "clean.txt",
        "staged.txt",
        "main.rs",
        "文件.rs",
        "flake.nix",
        "Cargo.toml",
        "Dockerfile",
        "README.md",
        "widget.tsx",
        "image.PNG",
        "archive.tar",
        "unknown.blob",
    ] {
        std::fs::write(root.join(name), format!("original contents of {name}\n"))?;
    }
    std::fs::write(root.join(".gitignore"), "ignored.txt\n")?;
    git(&root, &["add", "."])?;
    git(&root, &["commit", "-qm", "initial"])?;
    std::fs::write(
        root.join("changed/modified.txt"),
        "working tree modification\n",
    )?;
    std::fs::write(root.join("staged.txt"), "staged modification\n")?;
    std::fs::write(root.join("added.txt"), "added\n")?;
    std::fs::write(root.join("untracked.txt"), "untracked\n")?;
    std::fs::write(root.join("changed/child-added.txt"), "added child\n")?;
    std::fs::write(root.join("changed/child-new.txt"), "untracked child\n")?;
    std::fs::write(root.join("ignored.txt"), "ignored\n")?;
    git(
        &root,
        &["add", "staged.txt", "added.txt", "changed/child-added.txt"],
    )?;
    git(&root, &["rm", "-q", "removed/deleted.txt"])?;
    git(&root, &["mv", "moved/original.txt", "moved/renamed.txt"])?;

    let mut app = application()?;
    app.editor.theme =
        helix_view::theme::Loader::new(helix_loader::runtime_dirs()).load("catppuccin_mocha")?;
    // Reproduce a theme where modified diffs and directory names share the same blue.
    assert_eq!(
        app.editor.theme.get("diff.delta").fg,
        app.editor.theme.get("ui.text.directory").fg
    );
    assert_ne!(
        app.editor.theme.get("warning").fg,
        app.editor.theme.get("ui.text.directory").fg
    );
    let mut explorer = FileExplorer::new(root.clone(), &app.editor)?;
    let area = Rect::new(0, 0, 80, 25);
    explorer.layout(area, 40);
    // Expand changed/ to cover both file badges and a parent directory summary.
    explorer.expand(0, &app.editor)?;
    let expected = [
        ("changed", "MA?"),
        ("child-added.txt", "A"),
        ("child-new.txt", "?"),
        ("modified.txt", "M"),
        ("moved", "R"),
        ("removed", "D"),
        ("staged.txt", "M"),
        ("added.txt", "A"),
        ("untracked.txt", "?"),
        ("ignored.txt", ""),
        ("clean.txt", ""),
    ];
    let render = |explorer: &mut FileExplorer| {
        let mut surface = Surface::empty(area);
        explorer.render(&mut surface, &app.editor);
        let rows: Vec<_> = (1..area.height - 1)
            .map(|y| {
                let mut name = String::new();
                let mut x = 0;
                while x < 32 {
                    let cell = surface.get(x, y).unwrap();
                    name.push_str(&cell.symbol);
                    x += cell.width().max(1) as u16;
                }
                let badges: String = (32..39)
                    .map(|x| surface.get(x, y).unwrap().symbol.as_str())
                    .collect();
                (name, badges.trim().to_owned())
            })
            .collect();
        rows
    };
    let initial_status = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let rows = render(&mut explorer);
            if expected.iter().all(|(name, marker)| {
                rows.iter()
                    .any(|(text, badge)| text.trim_end().ends_with(name) && badge == marker)
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    anyhow::ensure!(
        initial_status.is_ok(),
        "unexpected sidebar rows: {:?}",
        render(&mut explorer)
    );

    let rows = render(&mut explorer);
    for (name, icon) in [
        ("changed", "\u{f07c}"),
        ("moved", "\u{f07b}"),
        ("main.rs", "\u{e7a8}"),
        ("文件.rs", "\u{e7a8}"),
        ("flake.nix", "\u{f313}"),
        ("Cargo.toml", "\u{e7a8}"),
        ("Dockerfile", "\u{e7b0}"),
        ("README.md", "\u{e73e}"),
        ("widget.tsx", "\u{e7ba}"),
        ("image.PNG", "\u{f1c5}"),
        ("archive.tar", "\u{f1c6}"),
        ("unknown.blob", "\u{f016}"),
    ] {
        let (row, _) = rows
            .iter()
            .find(|(row, _)| row.trim_end().ends_with(name))
            .unwrap_or_else(|| panic!("missing entry {name}: {rows:?}"));
        assert!(row.contains(icon), "missing file icon for {name}: {row}");
    }

    // Directories retain their own foreground; modified files and badges use a distinct color.
    for (name, scope) in [
        ("changed", "ui.text.directory"),
        ("modified.txt", "warning"),
        ("moved", "ui.text.directory"),
        ("removed", "ui.text.directory"),
        ("added.txt", "diff.plus"),
        ("untracked.txt", "diff.plus"),
        ("clean.txt", "ui.text"),
    ] {
        let index = explorer
            .entries
            .iter()
            .position(|entry| entry.path.file_name().unwrap() == name)
            .unwrap();
        explorer.selected = index;
        explorer.reveal_selection = true;
        for focused in [true, false] {
            explorer.focused = focused;
            let mut surface = Surface::empty(area);
            explorer.render(&mut surface, &app.editor);
            let x = (explorer.entries[index].depth * 2 + 4) as u16;
            let y = 1 + (index - explorer.offset) as u16;
            let cell = surface.get(x, y).unwrap();
            if let Some(foreground) = app.editor.theme.get(scope).fg {
                assert_eq!(cell.fg, foreground, "Git foreground for {name}");
            }
            if name == "changed" {
                let badge = surface.get(36, y).unwrap();
                assert_eq!(badge.fg, app.editor.theme.get("warning").fg.unwrap());
                assert_ne!(badge.fg, cell.fg);
                for x in [37, 38] {
                    assert_eq!(
                        surface.get(x, y).unwrap().fg,
                        app.editor.theme.get("diff.plus").fg.unwrap()
                    );
                }
            }
            let selection = if focused {
                "ui.selection.primary"
            } else {
                "ui.selection"
            };
            if let Some(background) = app.editor.theme.get(selection).bg {
                assert_eq!(cell.bg, background, "selection background for {name}");
            }
        }
    }
    explorer.focused = true;
    explorer.selected = 0;
    explorer.collapse();
    let rows = render(&mut explorer);
    assert!(rows
        .iter()
        .any(|(name, badges)| name.trim_end().ends_with("changed") && badges == "MA?"));
    explorer.expand(0, &app.editor)?;
    let mut surface = Surface::empty(area);
    explorer.render(&mut surface, &app.editor);
    let root_badges: String = (34..39)
        .map(|x| surface.get(x, 0).unwrap().symbol.as_str())
        .collect();
    assert_eq!(root_badges, "MRDA?");

    // Commit all changes, then refresh through the sidebar's normal key handler.
    git(&root, &["add", "-A"])?;
    git(&root, &["commit", "-qm", "changes"])?;
    let mut editor = app.editor;
    explorer.handle_event(&Event::Key(key!('R')), &mut editor);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let mut surface = Surface::empty(area);
            explorer.render(&mut surface, &editor);
            let clean = (0..area.height - 1)
                .all(|y| (32..39).all(|x| surface.get(x, y).unwrap().symbol.as_str() == " "));
            let expanded_child = (1..area.height - 1).any(|y| {
                let row: String = (0..37)
                    .map(|x| surface.get(x, y).unwrap().symbol.as_str())
                    .collect();
                row.contains("modified.txt")
            });
            if clean && expanded_child {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;

    // Icons can be disabled while the sidebar is open without changing the Git column.
    let mut updated_config = (*editor.config()).clone();
    updated_config.file_explorer.icons = false;
    editor.config = Arc::new(ArcSwap::from_pointee(updated_config));
    let mut surface = Surface::empty(area);
    explorer.render(&mut surface, &editor);
    for y in 0..area.height - 1 {
        for x in 0..39 {
            assert!(!surface
                .get(x, y)
                .unwrap()
                .symbol
                .chars()
                .any(|ch| ('\u{e000}'..='\u{f8ff}').contains(&ch)));
        }
        assert_eq!(surface.get(38, y).unwrap().symbol.as_str(), " ");
    }

    // Produce a real merge conflict and verify that it wins the directory summary.
    git(&root, &["checkout", "-qb", "sidebar-base"])?;
    git(&root, &["checkout", "-qb", "sidebar-side"])?;
    std::fs::write(root.join("clean.txt"), "side\n")?;
    git(&root, &["commit", "-qam", "side"])?;
    git(&root, &["checkout", "-q", "sidebar-base"])?;
    std::fs::write(root.join("clean.txt"), "base\n")?;
    git(&root, &["commit", "-qam", "base"])?;
    anyhow::ensure!(git(&root, &["merge", "--no-edit", "sidebar-side"]).is_err());
    std::fs::write(root.join("changed/modified.txt"), "modified again\n")?;
    std::fs::write(root.join("conflict-added.txt"), "added\n")?;
    std::fs::write(root.join("conflict-new.txt"), "untracked\n")?;
    git(&root, &["add", "conflict-added.txt"])?;
    git(&root, &["mv", "moved/renamed.txt", "moved/twice.txt"])?;
    git(&root, &["rm", "-q", "removed/keep.txt"])?;
    explorer.handle_event(&Event::Key(key!('R')), &mut editor);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let mut surface = Surface::empty(area);
            explorer.render(&mut surface, &editor);
            let conflict = (1..area.height - 1).any(|y| {
                let row: String = (0..37)
                    .map(|x| surface.get(x, y).unwrap().symbol.as_str())
                    .collect();
                row.contains("clean.txt") && surface.get(38, y).unwrap().symbol.as_str() == "!"
            });
            let root_badges: String = (33..39)
                .map(|x| surface.get(x, 0).unwrap().symbol.as_str())
                .collect();
            if conflict && root_badges == "!MRDA?" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;

    // All six states fit even at the minimum sidebar width without overwriting the separator.
    explorer.layout(area, 12);
    let mut surface = Surface::empty(area);
    explorer.render(&mut surface, &editor);
    let root_badges: String = (5..11)
        .map(|x| surface.get(x, 0).unwrap().symbol.as_str())
        .collect();
    assert_eq!(root_badges, "!MRDA?");
    assert_eq!(surface.get(11, 0).unwrap().symbol.as_str(), "│");
    Ok(())
}
