//! Wall-clock spans on the foreground editor task. All events share one logical
//! track, including time spent awaiting work on behalf of the current event.
use std::cell::RefCell;
use std::fs::File;
use std::future::Future;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use serde::Serialize;

const MAX_EVENTS: usize = 65_536;

tokio::task_local! {
    static RECORDING: RefCell<Recording>;
}

#[derive(Serialize)]
struct Recording {
    #[serde(skip)]
    origin: Instant,
    #[serde(skip)]
    pid: u32,
    #[serde(rename = "traceEvents")]
    events: Vec<Event>,
    #[serde(rename = "otherData")]
    stats: Stats,
}

#[derive(Default, Serialize)]
struct Stats {
    dropped_events: u64,
}

// Chrome Trace Event format, with timestamps and durations in microseconds.
#[derive(Serialize)]
struct Event {
    name: &'static str,
    ts: u64,
    dur: u64,
    ph: char,
    pid: u32,
    tid: u8,
}

/// Record a single editor session and export it when the editor returns.
pub async fn capture<T>(path: &Path, editor: impl Future<Output = Result<T>>) -> Result<T> {
    let file = File::create_new(path)
        .with_context(|| format!("could not create trace file {}", path.display()))?;
    let recording = Recording {
        origin: Instant::now(),
        pid: std::process::id(),
        events: Vec::with_capacity(MAX_EVENTS),
        stats: Stats::default(),
    };
    RECORDING
        .scope(RefCell::new(recording), async {
            let result = editor.await;
            let mut writer = BufWriter::new(file);
            let export = RECORDING
                .with(|cell| serde_json::to_writer(&mut writer, &*cell.borrow()))
                .map_err(std::io::Error::from)
                .and_then(|()| writer.flush())
                .with_context(|| format!("could not write trace file {}", path.display()));
            match (result, export) {
                (Err(err), Err(export)) => Err(err.context(format!("{export:#}"))),
                (result, Ok(())) => result,
                (Ok(_), Err(err)) => Err(err),
            }
        })
        .await
}

pub(crate) struct Span(usize);

#[must_use = "bind the span guard for the duration of the operation"]
pub(crate) fn span(name: &'static str) -> Option<Span> {
    RECORDING
        .try_with(|cell| {
            let mut rec = cell.borrow_mut();
            let index = rec.events.len();
            if index == MAX_EVENTS {
                rec.stats.dropped_events += 1;
                return None;
            }
            let ts = rec.origin.elapsed().as_micros() as u64;
            let pid = rec.pid;
            rec.events.push(Event {
                name,
                ts,
                dur: 0,
                ph: 'X',
                pid,
                tid: 1,
            });
            Some(Span(index))
        })
        .ok()
        .flatten()
}

impl Drop for Span {
    fn drop(&mut self) {
        RECORDING.with(|cell| {
            let mut rec = cell.borrow_mut();
            let end = rec.origin.elapsed().as_micros() as u64;
            let event = &mut rec.events[self.0];
            event.dur = end - event.ts;
        });
    }
}
