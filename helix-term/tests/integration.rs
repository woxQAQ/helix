#[cfg(feature = "integration")]
mod test {
    mod helpers;

    use helix_core::{syntax::config::AutoPairConfig, Selection};
    use helix_term::config::Config;

    use indoc::indoc;

    use self::helpers::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn hello_world() -> anyhow::Result<()> {
        test(("#[\n|]#", "ihello world<esc>", "hello world#[|\n]#")).await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn trace_editor_session() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("session.json");
        helix_term::trace::capture(&path, test(("#[\n|]#", "ix<esc>", "x#[|\n]#"))).await?;

        let trace: serde_json::Value = serde_json::from_reader(std::fs::File::open(path)?)?;
        let events = trace["traceEvents"].as_array().unwrap();
        let span_range = |event: &serde_json::Value| {
            let start = event["ts"].as_u64().unwrap();
            start..=start + event["dur"].as_u64().unwrap()
        };
        assert!(events.iter().any(|frame| {
            frame["name"] == "Application::render"
                && events.iter().any(|draw| {
                    draw["name"] == "Terminal::draw"
                        && span_range(frame).contains(span_range(draw).start())
                        && span_range(frame).contains(span_range(draw).end())
                })
        }));
        assert_eq!(trace["otherData"]["dropped_events"], 0);
        Ok(())
    }

    mod auto_pairs;
    mod bufferline;
    mod command_line;
    mod commands;
    mod context_menu;
    mod file_explorer;
    mod file_watcher;
    mod folding;
    mod git;
    mod inline_hunk;
    mod movement;
    mod splits;
    mod terminal;
}
