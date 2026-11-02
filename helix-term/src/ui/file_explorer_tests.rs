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
    explorer.handle_event(&Event::Key(key!('r')), &mut editor);
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
    explorer.handle_event(&Event::Key(key!('r')), &mut editor);
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
