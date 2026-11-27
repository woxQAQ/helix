use super::*;

use helix_term::ui::MarkdownPreview;
use helix_view::{current, current_ref, graphics::Rect};
use tui::buffer::Buffer;

fn screen(surface: &Buffer, area: Rect) -> String {
    let mut out = String::new();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            out.push_str(&surface[(x, y)].symbol);
        }
        out.push('\n');
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn toggle_command_reserves_and_releases_columns() -> anyhow::Result<()> {
    let mut app = helpers::AppBuilder::new()
        .with_input_text("#[|]# Title\n\nbody\n")
        .build()?;

    test_key_sequences(
        &mut app,
        vec![
            (
                Some(":markdown-preview-toggle<ret>"),
                Some(&|app| {
                    let (view, _) = current_ref!(app.editor);
                    assert_eq!(view.area.width, 60);
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
            (
                Some(":markdown-preview-toggle<ret>"),
                Some(&|app| {
                    let (view, _) = current_ref!(app.editor);
                    assert_eq!(view.area.width, 120);
                    helpers::assert_status_not_error(&app.editor);
                }),
            ),
        ],
        false,
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn preview_renders_markdown_and_follows_the_cursor() -> anyhow::Result<()> {
    // The body is long enough to wrap in the preview, so following the cursor has to count the
    // screen rows each rendered line takes, not just rendered lines.
    let sentence = "wrap the preview to check that screen rows are counted ".repeat(3);
    let text = (1..=30)
        .map(|section| format!("## Section {section}\n\n{sentence} {section}\n"))
        .collect::<String>();
    let mut app = helpers::AppBuilder::new()
        .with_input_text(format!("#[|]# {text}"))
        .build()?;

    let area = Rect::new(0, 0, 80, 12);
    let mut preview = MarkdownPreview::default();
    assert_eq!(preview.layout(area, 40), Rect::new(0, 0, 40, 12));
    let mut surface = Buffer::empty(area);

    preview.render(&mut surface, &app.editor);
    let first = screen(&surface, area);
    assert!(first.contains("Section 1"), "rendered:\n{first}");
    assert!(!first.contains("Section 30"), "rendered:\n{first}");
    // Headings render as their text, so the markdown markers must be gone.
    assert!(!first.contains("## Section"), "rendered:\n{first}");

    let (view, doc) = current!(app.editor);
    let text = doc.text().slice(..);
    let last_line = text.len_lines().saturating_sub(2);
    doc.set_selection(view.id, Selection::point(text.line_to_char(last_line)));

    preview.render(&mut surface, &app.editor);
    let last = screen(&surface, area);
    assert!(last.contains("Section 30"), "rendered:\n{last}");
    assert!(last.contains(" 30"), "rendered:\n{last}");
    assert!(!last.contains("Section 1\n"), "rendered:\n{last}");

    Ok(())
}
