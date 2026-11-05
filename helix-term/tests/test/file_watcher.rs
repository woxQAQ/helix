use super::*;

use std::{path::Path, process::Command, time::Duration};

use helix_term::application::Application;
use helix_view::{current_ref, input::parse_macro};

fn git(root: &Path, args: &[&str]) -> anyhow::Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .output()?;
    anyhow::ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

async fn keys(app: &mut Application, sequence: &str) -> anyhow::Result<()> {
    for key in parse_macro(sequence)? {
        #[cfg(not(windows))]
        let event = termina::event::Event::Key(termina::event::KeyEvent::from(key));
        #[cfg(windows)]
        let event = crossterm::event::Event::Key(crossterm::event::KeyEvent::from(key));
        app.handle_terminal_events(Ok(event)).await;
    }
    Ok(())
}

async fn until(app: &mut Application, ready: impl Fn(&Application) -> bool) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut input = futures_util::stream::pending();
        loop {
            app.event_loop_until_idle(&mut input).await;
            if ready(app) {
                break;
            }
        }
    })
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn external_atomic_replacement_updates_all_splits_and_survives_recreation(
) -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("file.txt");
    std::fs::write(&path, "first\nsecond\nthird\n")?;
    let mut app = helpers::AppBuilder::new().with_file(&path, None).build()?;
    keys(&mut app, "<C-w>vG").await?;
    // Let the initial watcher registration and snapshot settle before changing the file.
    until(&mut app, |_| true).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let replacement = root.path().join("replacement");
    std::fs::write(&replacement, "short\n")?;
    std::fs::rename(&replacement, &path)?;
    until(&mut app, |app| {
        current_ref!(app.editor).1.text().to_string() == "short\n"
    })
    .await?;
    assert!(!current_ref!(app.editor).1.is_modified());
    keys(&mut app, "<C-w>wggix<esc>u").await?;
    assert_eq!(current_ref!(app.editor).1.text().to_string(), "short\n");

    std::fs::remove_file(&path)?;
    until(&mut app, |_| true).await?;
    std::fs::write(&path, "recreated\n")?;
    until(&mut app, |app| {
        current_ref!(app.editor).1.text().to_string() == "recreated\n"
    })
    .await?;
    keys(&mut app, ":qa!<ret>").await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn worktree_commit_refreshes_hunk_base_without_overwriting_unsaved_edits(
) -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let repo = root.path().join("repo");
    let worktree = root.path().join("worktree");
    std::fs::create_dir(&repo)?;
    git(&repo, &["init", "-q"])?;
    std::fs::write(repo.join("file.txt"), "base\n")?;
    git(&repo, &["add", "."])?;
    git(&repo, &["commit", "-qm", "initial"])?;
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-qb",
            "linked",
            worktree.to_str().unwrap(),
        ],
    )?;
    let path = worktree.join("file.txt");
    let mut app = helpers::AppBuilder::new().with_file(&path, None).build()?;
    keys(&mut app, "iunsaved <esc>").await?;
    until(&mut app, |_| true).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    std::fs::write(&path, "committed\n")?;
    git(&worktree, &["add", "."])?;
    git(&worktree, &["commit", "-qm", "external"])?;
    until(&mut app, |app| {
        current_ref!(app.editor)
            .1
            .diff_handle()
            .is_some_and(|diff| diff.load().diff_base().to_string() == "committed\n")
    })
    .await?;
    let (_, doc) = current_ref!(app.editor);
    assert_eq!(doc.text().to_string(), "unsaved base\n");
    assert!(doc.is_modified());
    assert!(app
        .editor
        .get_status()
        .is_some_and(|(message, _)| message.contains("unsaved edits preserved")));
    git(&worktree, &["switch", "-qc", "next"])?;
    until(&mut app, |app| {
        current_ref!(app.editor)
            .1
            .version_control_head()
            .is_some_and(|head| head.as_ref().as_ref() == "next")
    })
    .await?;
    keys(&mut app, ":qa!<ret>").await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn explorer_discovers_external_files_without_manual_refresh() -> anyhow::Result<()> {
    let root = tempfile::Builder::new()
        .prefix("watcher-explorer-")
        .tempdir_in(helix_loader::find_workspace().0)?;
    let first = root.path().join("a.txt");
    let second = root.path().join("z.txt");
    std::fs::write(&first, "initial\n")?;
    let mut app = helpers::AppBuilder::new().with_file(&first, None).build()?;
    keys(&mut app, "<space>e").await?;
    until(&mut app, |_| true).await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    std::fs::write(&second, "new file\n")?;
    std::fs::write(&first, "updated\n")?;
    until(&mut app, |app| {
        current_ref!(app.editor).1.text().to_string() == "updated\n"
    })
    .await?;
    keys(&mut app, "j<ret>").await?;
    assert_eq!(current_ref!(app.editor).1.path(), Some(second.as_path()));
    assert_eq!(current_ref!(app.editor).1.text().to_string(), "new file\n");
    keys(&mut app, ":qa!<ret>").await?;
    Ok(())
}
