//! Filesystem events invalidate disk and VCS snapshots; applying them stays on the editor thread.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::{compositor::Compositor, job::Jobs, ui::EditorView};
use helix_view::{document::from_reader, Editor};

pub(crate) struct FileWatcher {
    roots: Arc<Mutex<HashSet<(PathBuf, bool)>>>,
    registrations: std::sync::mpsc::SyncSender<()>,
    stopped: Arc<AtomicBool>,
    directories: HashMap<PathBuf, Vec<(PathBuf, bool)>>,
    trigger: mpsc::Sender<()>,
    events: mpsc::Receiver<()>,
    worker: tokio::task::JoinHandle<()>,
    generation: Arc<AtomicU64>,
    conflicts: Arc<Mutex<HashMap<PathBuf, std::time::SystemTime>>>,
}

impl FileWatcher {
    pub(crate) fn new() -> Self {
        // Both channels are bounded: bursts invalidate a snapshot once, regardless of event count.
        let (trigger, mut raw_events) = mpsc::channel(1);
        let (ready, events) = mpsc::channel(1);
        let roots = Arc::new(Mutex::new(HashSet::new()));
        let (registrations, requests) = std::sync::mpsc::sync_channel(1);
        let stopped = Arc::new(AtomicBool::new(false));
        let registration_roots = roots.clone();
        let registration_stop = stopped.clone();
        let registration_sender = registrations.clone();
        let retired = Arc::new(Mutex::new(HashSet::<PathBuf>::new()));
        let retired_events = retired.clone();
        let sender = trigger.clone();
        std::thread::spawn(move || {
            let changes = sender.clone();
            let mut watcher: RecommendedWatcher =
                match notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    match event {
                        Ok(event) if matches!(event.kind, notify::EventKind::Access(_)) => (),
                        Ok(event) => {
                            if matches!(
                                event.kind,
                                notify::EventKind::Remove(_)
                                    | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
                            ) {
                                retired_events
                                    .lock()
                                    .unwrap()
                                    .extend(event.paths.iter().cloned());
                            }
                            if matches!(
                                event.kind,
                                notify::EventKind::Create(_)
                                    | notify::EventKind::Remove(_)
                                    | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
                            ) {
                                let _ = registration_sender.try_send(());
                            }
                            let _ = changes.try_send(());
                        }
                        Err(err) => {
                            log::warn!("file watcher: {err}");
                            let _ = changes.try_send(());
                        }
                    }
                }) {
                    Ok(watcher) => watcher,
                    Err(err) => {
                        log::warn!("could not start file watcher: {err}");
                        return;
                    }
                };
            let mut watched = HashSet::<PathBuf>::new();
            while requests.recv().is_ok() {
                if registration_stop.load(Ordering::Relaxed) {
                    break;
                }
                let roots: HashSet<(PathBuf, bool)> = registration_roots.lock().unwrap().clone();
                let mut desired = HashSet::new();
                for (root, recursive) in roots {
                    desired.insert(root.clone());
                    if recursive {
                        // Register directories individually so ignored build output consumes no watches.
                        for entry in ignore::WalkBuilder::new(&root)
                            .hidden(false)
                            .filter_entry(|entry| entry.file_name() != ".git")
                            .build()
                            .filter_map(Result::ok)
                        {
                            if entry.file_type().is_some_and(|kind| kind.is_dir()) {
                                desired.insert(entry.into_path());
                            }
                        }
                    }
                }
                let retired = std::mem::take(&mut *retired.lock().unwrap());
                for path in &watched {
                    if retired.iter().any(|removed| path.starts_with(removed)) {
                        let _ = watcher.unwatch(path);
                    }
                }
                watched.retain(|path| !retired.iter().any(|removed| path.starts_with(removed)));
                for path in watched.difference(&desired) {
                    let _ = watcher.unwatch(path);
                }
                watched.retain(|path| desired.contains(path));
                let added: Vec<_> = desired.difference(&watched).cloned().collect();
                for path in added {
                    match watcher.watch(&path, RecursiveMode::NonRecursive) {
                        Ok(()) => (),
                        Err(err) => {
                            log::debug!("could not watch {}: {err}", path.display());
                            continue;
                        }
                    }
                    watched.insert(path);
                }
                let _ = sender.try_send(());
            }
        });
        let worker = tokio::spawn(async move {
            while raw_events.recv().await.is_some() {
                tokio::time::sleep(Duration::from_millis(200)).await;
                while raw_events.try_recv().is_ok() {}
                if ready.send(()).await.is_err() {
                    break;
                }
            }
        });
        Self {
            roots,
            registrations,
            stopped,
            directories: HashMap::new(),
            trigger,
            events,
            worker,
            generation: Arc::new(AtomicU64::new(0)),
            conflicts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) fn request_refresh(&self) {
        let _ = self.trigger.try_send(());
    }

    pub(crate) async fn next(&mut self) -> Option<()> {
        self.events.recv().await
    }

    pub(crate) fn update(&mut self, editor: &Editor, compositor: &mut Compositor) {
        let mut directories: HashSet<_> = editor
            .documents()
            .filter_map(|doc| doc.path()?.parent().map(ToOwned::to_owned))
            .collect();
        let explorer_root = compositor
            .find::<EditorView>()
            .and_then(|view| view.file_explorer_root())
            .map(ToOwned::to_owned);
        if let Some(root) = &explorer_root {
            directories.insert(root.clone());
        }
        self.directories
            .retain(|path, _| directories.contains(path));
        for directory in directories {
            self.directories
                .entry(directory.clone())
                .or_insert_with(|| editor.diff_providers.watch_paths(&directory));
        }
        let mut desired: HashSet<_> = self.directories.values().flatten().cloned().collect();
        // Explicit document parents remain watched even when ignored by the workspace walker.
        desired.extend(self.directories.keys().cloned().map(|path| (path, false)));
        if let Some(root) = explorer_root {
            desired.insert((root, true));
        }
        let mut roots = self.roots.lock().unwrap();
        if *roots != desired {
            *roots = desired;
            let _ = self.registrations.try_send(());
        }
    }

    pub(crate) fn refresh(&self, editor: &Editor, jobs: &mut Jobs) {
        // Saves publish their own disk state. A subsequent save completion requests another scan.
        if editor.write_count != 0 {
            return;
        }
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let latest = self.generation.clone();
        let conflicts = self.conflicts.clone();
        let providers = editor.diff_providers.clone();
        let documents: Vec<_> = editor
            .documents()
            .filter_map(|doc| {
                Some((
                    doc.id(),
                    doc.path()?.to_owned(),
                    doc.version(),
                    doc.last_saved_time(),
                    doc.is_modified(),
                    doc.encoding(),
                    editor
                        .workspace_trust
                        .query(
                            doc.workspace_root(),
                            helix_loader::workspace_trust::TrustQuery::Git,
                        )
                        .is_trusted(),
                ))
            })
            .collect();
        jobs.callback(async move {
            let snapshots = tokio::task::spawn_blocking(move || {
                documents
                    .into_iter()
                    .map(
                        |(id, path, version, previous_save, was_modified, encoding, trusted)| {
                            let disk = (|| -> anyhow::Result<_> {
                                let mut file = std::fs::File::open(&path)?;
                                let before = file.metadata()?.modified()?;
                                if before == previous_save {
                                    return Ok(None);
                                }
                                let (text, ..) = from_reader(&mut file, Some(encoding))?;
                                anyhow::ensure!(
                                    before == std::fs::metadata(&path)?.modified()?,
                                    "file changed during read"
                                );
                                Ok(Some((text, before)))
                            })();
                            let base = providers.get_diff_base(&path, trusted);
                            let head = providers.get_current_head_name(&path, trusted);
                            (
                                id,
                                path,
                                version,
                                previous_save,
                                was_modified,
                                disk,
                                base,
                                head,
                            )
                        },
                    )
                    .collect::<Vec<_>>()
            })
            .await?;
            Ok(crate::job::Callback::EditorCompositor(Box::new(
                move |editor, compositor| {
                    if latest.load(Ordering::Relaxed) != generation || editor.write_count != 0 {
                        return;
                    }
                    let scrolloff = editor.config().scrolloff;
                    let mut conflicts = conflicts.lock().unwrap();
                    conflicts.retain(|path, _| editor.document_id_by_path(path).is_some());
                    for (id, path, version, previous_save, was_modified, disk, base, head) in
                        snapshots
                    {
                        let Some(doc) = editor.document_mut(id) else {
                            continue;
                        };
                        if doc.path() != Some(path.as_path()) {
                            continue;
                        }
                        doc.refresh_diff_base(base, head);
                        doc.detect_readonly();
                        if !doc.is_modified() {
                            conflicts.remove(&path);
                        }
                        let (text, saved_time) = match disk {
                            Ok(Some(disk)) => disk,
                            Ok(None) => continue,
                            Err(err) => {
                                log::debug!("could not refresh {}: {err}", path.display());
                                continue;
                            }
                        };
                        if doc.version() != version || doc.last_saved_time() != previous_save {
                            continue;
                        }
                        if doc.is_modified() {
                            if saved_time != doc.last_saved_time()
                                && doc.text() != &text
                                && conflicts.insert(path.clone(), saved_time) != Some(saved_time)
                            {
                                editor.set_warning(format!(
                                    "{} changed on disk; unsaved edits preserved",
                                    path.display()
                                ));
                            }
                            continue;
                        }
                        if was_modified {
                            continue;
                        }
                        let changed = doc.text() != &text;
                        let view_id = doc
                            .selections()
                            .keys()
                            .next()
                            .copied()
                            .unwrap_or(editor.tree.focus);
                        let view = helix_view::view_mut!(editor, view_id);
                        let doc = helix_view::doc_mut!(editor, &id);
                        doc.ensure_view_init(view_id);
                        view.sync_changes(doc);
                        doc.reload_text(view, text, saved_time);
                        for (view, _) in editor.tree.views_mut() {
                            let doc = helix_view::doc_mut!(editor, &id);
                            view.sync_changes(doc);
                            if view.doc == id {
                                view.ensure_cursor_in_view(doc, scrolloff);
                            }
                        }
                        if changed {
                            editor
                                .language_servers
                                .file_event_handler
                                .file_changed(path);
                        }
                    }
                    if let Some(view) = compositor.find::<EditorView>() {
                        view.refresh_file_explorer(editor);
                    }
                    helix_event::request_redraw();
                },
            )))
        });
    }
}

impl Drop for FileWatcher {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        let _ = self.registrations.try_send(());
        self.worker.abort();
    }
}
