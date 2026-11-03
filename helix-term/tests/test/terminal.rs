use super::*;

use std::time::Duration;

use helix_term::{
    compositor::{Compositor, Context, Event},
    job::Jobs,
    ui::terminal::FloatingTerminal,
};
use helix_view::{
    graphics::{Color, Rect},
    input::{parse_macro, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
};
use portable_pty::CommandBuilder;
use tui::buffer::Buffer;

// Exercise the complete PTY-to-compositor path with a deterministic child process.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn floating_terminal_forwards_input_renders_and_exits() -> anyhow::Result<()> {
    let mut jobs = Jobs::new();
    let mut app = helpers::AppBuilder::new().build()?;
    let area = Rect::new(0, 0, 100, 30);
    let mut compositor = Compositor::new(area);
    let mut command = CommandBuilder::new("sh");
    command.args(["-c", r"stty raw -echo; printf '\033[?1h\033[?2004h\033[?1000h\033[?1006h\033[31mREADY 界\033[0m\r\n'; dd bs=1 count=7 2>/dev/null | od -An -tx1; printf '\r\nKEYS\r\n'; dd bs=1 count=14 2>/dev/null | od -An -tx1; printf '\r\nPASTE\r\n'; dd bs=1 count=9 2>/dev/null | od -An -tx1; printf '\r\nMOUSE\r\n'; dd bs=1 count=1 2>/dev/null; stty size; printf '\r\nRESIZED\r\n'; dd bs=1 count=1 2>/dev/null"]);
    compositor.push(Box::new(FloatingTerminal::new(command, area, None)?));
    let mut cx = Context {
        editor: &mut app.editor,
        scroll: None,
        jobs: &mut jobs,
    };
    let mut surface = Buffer::empty(area);

    async fn wait_for(
        compositor: &mut Compositor,
        cx: &mut Context<'_>,
        surface: &mut Buffer,
        text: &str,
    ) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                compositor.render(surface.area, surface, cx);
                let rendered: String = surface
                    .content
                    .iter()
                    .map(|cell| cell.symbol.as_str())
                    .collect();
                if rendered.contains(text) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("terminal did not render {text:?}"));
    }
    wait_for(&mut compositor, &mut cx, &mut surface, "READY 界").await;
    assert_eq!(surface[(6, 2)].fg, helix_view::graphics::Color::Indexed(1));
    for key in parse_macro("<up><S-tab><esc>")? {
        compositor.handle_event(&Event::Key(key), &mut cx);
    }
    wait_for(&mut compositor, &mut cx, &mut surface, "KEYS").await;
    let rendered: String = surface
        .content
        .iter()
        .map(|cell| cell.symbol.as_str())
        .collect();
    assert!(rendered.contains("1b 4f 41 1b 5b 5a 1b"));
    compositor.handle_event(&Event::Paste("hi".into()), &mut cx);
    wait_for(&mut compositor, &mut cx, &mut surface, "PASTE").await;
    let rendered: String = surface
        .content
        .iter()
        .map(|cell| cell.symbol.as_str())
        .collect();
    assert!(rendered.contains("1b 5b 32 30 30 7e 68 69 1b 5b 32 30 31 7e"));
    compositor.handle_event(
        &Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 8,
            row: 3,
            modifiers: KeyModifiers::empty(),
        }),
        &mut cx,
    );
    wait_for(&mut compositor, &mut cx, &mut surface, "MOUSE").await;
    let tiny = Rect::new(0, 0, 1, 1);
    compositor.resize(tiny);
    let mut tiny_surface = Buffer::empty(tiny);
    compositor.render(tiny, &mut tiny_surface, &mut cx);
    assert!(compositor.cursor(tiny, cx.editor).0.is_none());
    let resized = Rect::new(0, 0, 80, 24);
    compositor.resize(resized);
    surface = Buffer::empty(resized);
    compositor.render(resized, &mut surface, &mut cx);
    compositor.handle_event(&Event::Key(parse_macro("x")?[0]), &mut cx);
    wait_for(&mut compositor, &mut cx, &mut surface, "RESIZED").await;
    let rendered: String = surface
        .content
        .iter()
        .map(|cell| cell.symbol.as_str())
        .collect();
    assert!(rendered.contains("20 70"));
    compositor.handle_event(&Event::Key(parse_macro("q")?[0]), &mut cx);
    let callback = tokio::time::timeout(Duration::from_secs(10), cx.jobs.callbacks.recv())
        .await?
        .unwrap();
    cx.jobs
        .handle_callback(cx.editor, &mut compositor, Ok(Some(callback)));
    assert_eq!(compositor.layer_count(), 0);
    helpers::assert_status_not_error(cx.editor);
    Ok(())
}

#[cfg(unix)]
/// Runs `script` as the child of a floating terminal opened on `background` and
/// returns what the terminal rendered for it.
async fn child_output(background: Option<Color>, script: &str) -> anyhow::Result<String> {
    let mut jobs = Jobs::new();
    let mut app = helpers::AppBuilder::new().build()?;
    let area = Rect::new(0, 0, 100, 30);
    let mut compositor = Compositor::new(area);
    let mut command = CommandBuilder::new("sh");
    command.args([
        "-c",
        &format!("stty raw -echo; {script}; printf '\r\nDONE\r\n'; dd bs=1 count=1 2>/dev/null"),
    ]);
    compositor.push(Box::new(FloatingTerminal::new(command, area, background)?));
    let mut cx = Context {
        editor: &mut app.editor,
        scroll: None,
        jobs: &mut jobs,
    };
    let mut surface = Buffer::empty(area);
    let rendered = |surface: &Buffer| -> String {
        surface
            .content
            .iter()
            .map(|cell| cell.symbol.as_str())
            .collect()
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            compositor.render(surface.area, &mut surface, &mut cx);
            if rendered(&surface).contains("DONE") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    Ok(rendered(&surface))
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn floating_terminal_answers_queries_split_across_reads() -> anyhow::Result<()> {
    let rendered = child_output(
        Some(Color::Rgb(30, 30, 46)),
        r"printf '\033]1'; sleep 0.2; printf '1;?\007'; head -c 24 | od -An -tx1 | tr -d ' \n'",
    )
    .await?;
    assert!(
        rendered.contains("1b5d31313b7267623a316531652f316531652f3265326507"),
        "{rendered}"
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn floating_terminal_answers_terminal_queries() -> anyhow::Result<()> {
    // tcell-based children (lazygit) block for a second when the terminal does not
    // answer the device attributes query.
    let rendered = child_output(
        None,
        r"printf '\033[c\033[18t'; head -c 7 | od -An -tx1; head -c 4 | od -An -tx1",
    )
    .await?;
    assert!(rendered.contains("1b 5b 3f 31 3b 32 63"), "{rendered}");
    assert!(rendered.contains("1b 5b 38 3b"), "{rendered}");
    Ok(())
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn floating_terminal_reports_its_background_to_the_child() -> anyhow::Result<()> {
    // The color scheme report (`?997;1n` dark, `?997;2n` light) and the background
    // color report, hex encoded without separators by `od` and `tr`.
    for (background, replies) in [
        (
            Color::Rgb(30, 30, 46),
            concat!(
                "1b5b3f3939373b316e",
                "1b5d31313b7267623a316531652f316531652f3265326507"
            ),
        ),
        (
            Color::Rgb(0xfd, 0xf6, 0xe3),
            concat!(
                "1b5b3f3939373b326e",
                "1b5d31313b7267623a666466642f663666362f6533653307"
            ),
        ),
    ] {
        let rendered = child_output(
            Some(background),
            r"printf '\033[?996n'; head -c 9 | od -An -tx1 | tr -d ' \n'; printf '\033]11;?\007'; head -c 24 | od -An -tx1 | tr -d ' \n'",
        )
        .await?;
        assert!(rendered.contains(replies), "{rendered}");
    }
    Ok(())
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn floating_terminal_close_terminates_child_and_handles_startup_errors() -> anyhow::Result<()>
{
    let mut jobs = Jobs::new();
    let mut app = helpers::AppBuilder::new().build()?;
    let area = Rect::new(0, 0, 80, 24);
    let mut compositor = Compositor::new(area);
    let mut command = CommandBuilder::new("sh");
    command.args(["-c", "exec cat"]);
    compositor.push(Box::new(FloatingTerminal::new(command, area, None)?));
    let mut cx = Context {
        editor: &mut app.editor,
        scroll: None,
        jobs: &mut jobs,
    };
    compositor.handle_event(&Event::Key(parse_macro("<C-g>")?[0]), &mut cx);
    assert_eq!(compositor.layer_count(), 0);
    // A replacement survives completion of the closed process.
    let mut command = CommandBuilder::new("sh");
    command.args(["-c", "exec cat"]);
    compositor.push(Box::new(FloatingTerminal::new(command, area, None)?));
    let callback = tokio::time::timeout(Duration::from_secs(10), cx.jobs.callbacks.recv())
        .await?
        .unwrap();
    cx.jobs
        .handle_callback(cx.editor, &mut compositor, Ok(Some(callback)));
    assert_eq!(compositor.layer_count(), 1);
    compositor.handle_event(&Event::Key(parse_macro("<C-g>")?[0]), &mut cx);
    let callback = tokio::time::timeout(Duration::from_secs(10), cx.jobs.callbacks.recv())
        .await?
        .unwrap();
    cx.jobs
        .handle_callback(cx.editor, &mut compositor, Ok(Some(callback)));
    helpers::assert_status_not_error(cx.editor);
    let mut command = CommandBuilder::new("sh");
    command.args(["-c", "exit 7"]);
    compositor.push(Box::new(FloatingTerminal::new(command, area, None)?));
    let callback = tokio::time::timeout(Duration::from_secs(10), cx.jobs.callbacks.recv())
        .await?
        .unwrap();
    cx.jobs
        .handle_callback(cx.editor, &mut compositor, Ok(Some(callback)));
    assert_eq!(compositor.layer_count(), 0);
    assert!(cx.editor.get_status().unwrap().0.contains("7"));
    assert!(FloatingTerminal::new(
        CommandBuilder::new("helix-nonexistent-terminal-command"),
        area,
        None
    )
    .is_err());
    assert!(FloatingTerminal::new(CommandBuilder::new("sh"), Rect::new(0, 0, 1, 1), None).is_err());
    Ok(())
}
