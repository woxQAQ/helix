use super::*;

use std::time::Duration;

use helix_term::{
    compositor::{Compositor, Context, Event},
    job::Jobs,
    ui::terminal::FloatingTerminal,
};
use helix_view::{
    graphics::Rect,
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
    compositor.push(Box::new(FloatingTerminal::new(command, area)?));
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
#[tokio::test(flavor = "multi_thread")]
async fn floating_terminal_close_terminates_child_and_handles_startup_errors() -> anyhow::Result<()>
{
    let mut jobs = Jobs::new();
    let mut app = helpers::AppBuilder::new().build()?;
    let area = Rect::new(0, 0, 80, 24);
    let mut compositor = Compositor::new(area);
    let mut command = CommandBuilder::new("sh");
    command.args(["-c", "exec cat"]);
    compositor.push(Box::new(FloatingTerminal::new(command, area)?));
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
    compositor.push(Box::new(FloatingTerminal::new(command, area)?));
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
    compositor.push(Box::new(FloatingTerminal::new(command, area)?));
    let callback = tokio::time::timeout(Duration::from_secs(10), cx.jobs.callbacks.recv())
        .await?
        .unwrap();
    cx.jobs
        .handle_callback(cx.editor, &mut compositor, Ok(Some(callback)));
    assert_eq!(compositor.layer_count(), 0);
    assert!(cx.editor.get_status().unwrap().0.contains("7"));
    assert!(FloatingTerminal::new(
        CommandBuilder::new("helix-nonexistent-terminal-command"),
        area
    )
    .is_err());
    assert!(FloatingTerminal::new(CommandBuilder::new("sh"), Rect::new(0, 0, 1, 1)).is_err());
    Ok(())
}
