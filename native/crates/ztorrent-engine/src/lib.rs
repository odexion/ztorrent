//! The ztorrent engine: one thread that owns the libtorrent session, the
//! library of torrents and the state file, and answers the window only through
//! the command set in `ztorrent_core::command`.
//!
//! It is the port of electron/engine.js, egress.js and part-store.js. The rules
//! those files wrote down -- fail closed, snapshot what the UI needs onto the
//! record, one helper for the effective rates -- are kept here, with the comments
//! that explain them.

mod config;
mod details;
mod net;
mod paths;
mod torrent;

pub use details::share_ratio;
pub use net::{fetch_torrent, list_interfaces};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use torrent::{PART_SUFFIX, Torrent, plan_parts};
use ztorrent_core::command::*;
use ztorrent_core::egress::{self, EgressPolicy};
use ztorrent_core::settings::{self, Settings};
use ztorrent_core::*;
use ztorrent_lt_sys as lt;

const TICK: Duration = Duration::from_secs(1);
const RESUME_EVERY: Duration = Duration::from_secs(300);
const SNAPSHOT_EVERY: Duration = Duration::from_secs(30);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

pub struct EngineOptions {
    /// Where the state file lives; resume data goes beside it.
    pub data_dir: PathBuf,
    pub version: String,
}

pub struct EngineHandle {
    pub commands: flume::Sender<Command>,
    pub events: flume::Receiver<Event>,
    pub thread: std::thread::JoinHandle<()>,
}

/// Starts the engine thread. Fails if the session cannot be created.
pub fn spawn(store: Store, options: EngineOptions) -> anyhow::Result<EngineHandle> {
    let (cmd_tx, cmd_rx) = flume::unbounded::<Command>();
    let (ev_tx, ev_rx) = flume::unbounded::<Event>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let thread = std::thread::Builder::new().name("ztorrent-engine".into()).spawn(move || {
        match Engine::start(store, options, ev_tx) {
            Ok(mut engine) => {
                let _ = ready_tx.send(Ok(()));
                engine.run(cmd_rx);
            }
            Err(err) => {
                let _ = ready_tx.send(Err(format!("{err:#}")));
            }
        }
    })?;
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(EngineHandle { commands: cmd_tx, events: ev_rx, thread }),
        Ok(Err(e)) => Err(anyhow::anyhow!(e)),
        Err(_) => Err(anyhow::anyhow!("the engine thread exited while starting")),
    }
}

/// Work finished on a helper thread, handed back to the engine thread.
enum Internal {
    Log(String, LogLevel),
    Fetched { bytes: Result<Vec<u8>, String>, options: AddOptions, reply: Option<Reply<Result<AddOutcome, String>>> },
    Created { options: CreateTorrentOptions, bytes: Vec<u8> },
}

pub(crate) struct Engine {
    pub(crate) store: Store,
    pub(crate) session: lt::cxx::UniquePtr<lt::Session>,
    pub(crate) torrents: Vec<Torrent>,
    by_handle: HashMap<u64, String>,
    events: flume::Sender<Event>,
    internal_tx: flume::Sender<Internal>,
    internal_rx: flume::Receiver<Internal>,
    log: VecDeque<LogLine>,
    selected: Option<String>,
    /// Preferences as they were at start-up: the ones that need a restart are
    /// always read from here.
    pub(crate) started: Settings,
    pub(crate) policy: Option<EgressPolicy>,
    pub(crate) listen_port: u16,
    resume_dir: PathBuf,
    version: String,
    order_seq: i64,
    next_tick: Instant,
    next_resume: Instant,
    next_snapshot: Instant,
    next_interface: Instant,
    /// None until the bound interface has been looked at once.
    bind_address: Option<Option<String>>,
    suspended: Vec<String>,
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl Engine {
    fn start(store: Store, options: EngineOptions, events: flume::Sender<Event>) -> anyhow::Result<Engine> {
        let settings = store.settings().clone();
        // Decided before the session exists: the policy decides what the session
        // is even allowed to switch on.
        let policy = EgressPolicy::from_settings(&settings);
        let config = config::session_config(&settings, &settings, policy.as_ref(), &options.version);
        let session = lt::new_session(&config).map_err(|e| anyhow::anyhow!("could not start the torrent session: {}", e.what()))?;
        let (internal_tx, internal_rx) = flume::unbounded();
        let now = Instant::now();
        let mut engine = Engine {
            store,
            session,
            torrents: Vec::new(),
            by_handle: HashMap::new(),
            events,
            internal_tx,
            internal_rx,
            log: VecDeque::new(),
            selected: None,
            started: settings,
            policy,
            listen_port: 0,
            resume_dir: options.data_dir.join("resume"),
            version: options.version,
            order_seq: 0,
            next_tick: now,
            next_resume: now + RESUME_EVERY,
            next_snapshot: now + SNAPSHOT_EVERY,
            next_interface: now,
            bind_address: None,
            suspended: Vec::new(),
        };
        engine.log_start();
        engine.restore_session();
        Ok(engine)
    }

    fn log_start(&mut self) {
        self.log("ztorrent started. Listening for peers.", LogLevel::Info);
        if self.started.encryption == 2 {
            self.log(
                "Requiring protocol encryption: peers that will not encrypt are refused. This shrinks the pool of usable peers.",
                LogLevel::Info,
            );
        }
        if let Some(policy) = self.policy.clone() {
            let addr = policy.bind.as_deref().and_then(net::bind_address).map(|a| a.to_string());
            self.log(
                format!("Trackers, web seeds and peer connections go out via {}.", policy.describe(addr.as_deref())),
                LogLevel::Info,
            );
            let mut off = vec!["local discovery", "uTP", "port mapping", "udp:// trackers"];
            if policy.proxy.is_some() {
                off.insert(0, "DHT");
            }
            self.log(format!("{} are off while this is on -- none of them can be routed.", off.join(", ")), LogLevel::Info);
            if policy.proxy.is_some() {
                self.log("Inbound listeners closed: with a proxy, connections are outgoing only.", LogLevel::Info);
            }
        }
    }

    // --------------------------------------------------------------- loop

    fn run(&mut self, commands: flume::Receiver<Command>) {
        loop {
            loop {
                match commands.try_recv() {
                    Ok(Command::Shutdown(reply)) => {
                        self.shutdown();
                        let _ = reply.send(());
                        return;
                    }
                    Ok(cmd) => self.handle(cmd),
                    Err(flume::TryRecvError::Empty) => break,
                    Err(flume::TryRecvError::Disconnected) => {
                        self.shutdown();
                        return;
                    }
                }
            }
            while let Ok(msg) = self.internal_rx.try_recv() {
                self.handle_internal(msg);
            }
            if self.session.pin_mut().wait_for_alert(50) {
                self.drain_alerts();
            }
            let now = Instant::now();
            if self.policy.as_ref().is_some_and(|p| p.bind.is_some()) && now >= self.next_interface {
                self.next_interface = now + Duration::from_secs(1);
                self.check_interface();
            }
            if now >= self.next_tick {
                self.next_tick = now + TICK;
                self.tick();
            }
            if now >= self.next_resume {
                self.next_resume = now + RESUME_EVERY;
                for t in &self.torrents {
                    if let (Some(h), true) = (t.handle, t.live()) {
                        self.session.save_resume_data(h);
                    }
                }
            }
            self.store.flush_if_due();
        }
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    pub(crate) fn log(&mut self, message: impl Into<String>, level: LogLevel) {
        let line = LogLine { time: now_ms(), message: message.into(), level };
        self.log.push_back(line.clone());
        while self.log.len() > LOG_LIMIT {
            self.log.pop_front();
        }
        self.emit(Event::Log(line));
    }

    fn index(&self, id: &str) -> Option<usize> {
        self.torrents.iter().position(|t| t.id() == id)
    }

    fn index_of_handle(&self, handle: u64) -> Option<usize> {
        let id = self.by_handle.get(&handle)?.clone();
        self.index(&id)
    }

    fn tick(&mut self) {
        let before: Vec<State> = self.torrents.iter().map(|t| t.state()).collect();
        self.refresh_status();
        // A state the list shows differently -- checking done, a download become a
        // seed -- is written straight away rather than at the next snapshot.
        if self.torrents.iter().zip(&before).any(|(t, b)| t.state() != *b) {
            self.persist();
        }
        self.enforce_seed_goals();
        self.session.pin_mut().post_session_stats();
        if Instant::now() >= self.next_snapshot {
            self.next_snapshot = Instant::now() + SNAPSHOT_EVERY;
            for i in 0..self.torrents.len() {
                self.sync_record(i, true);
            }
            self.persist();
        }
        self.send_tick();
    }

    fn send_tick(&self) {
        let rows = self.rows();
        let globals = self.globals();
        let details = self.selected.as_deref().and_then(|id| self.details(id)).map(Box::new);
        self.emit(Event::Tick { rows, globals, details });
    }

    fn refresh_status(&mut self) {
        for t in &mut self.torrents {
            let Some(h) = t.handle else { continue };
            t.status = self.session.status(h);
            if t.status.valid {
                t.up_at_spawn.get_or_insert(t.status.all_time_upload);
                t.down_at_spawn.get_or_insert(t.status.all_time_download);
            }
        }
    }

    // ------------------------------------------------------------ session

    fn restore_session(&mut self) {
        let saved = std::mem::take(&mut self.store.data.torrents);
        let count = saved.len();
        for mut rec in saved {
            if rec.id.is_empty() {
                rec.id = uuid::Uuid::new_v4().to_string();
            }
            if rec.name.is_empty() {
                rec.name = "Unknown".into();
            }
            if rec.added_on == 0 {
                rec.added_on = now_ms();
            }
            let order = rec.order.unwrap_or(self.order_seq);
            rec.order = Some(order);
            self.order_seq = self.order_seq.max(order + 1);
            let mut t = Torrent::new(rec);
            t.want_start = !matches!(t.rec.state, State::Stopped | State::Paused);
            if !t.want_start && t.rec.state != State::Paused {
                t.rec.state = State::Stopped;
            }
            self.torrents.push(t);
        }
        if count > 0 {
            self.log(format!("Restored {count} torrent(s) from the previous session."), LogLevel::Info);
        }
        // Through the queue, so a restart never starts more than the limits allow.
        self.pump_queue();
        self.persist();
    }

    /// Copies what only a running torrent knows onto its record, so the tabs keep
    /// working when it stops. `heavy` also takes the piece map and file list.
    fn sync_record(&mut self, i: usize, heavy: bool) {
        let t = &self.torrents[i];
        let Some(h) = t.handle.filter(|_| t.live()) else { return };
        let (done, length, has_meta) = (t.done(), t.length(), t.status.has_metadata);
        let name = t.status.name.clone();
        let t = &mut self.torrents[i];
        t.rec.progress = done;
        t.rec.length = length;
        if has_meta && !name.is_empty() {
            t.rec.name = name;
        }
        if !heavy || !has_meta {
            return;
        }
        let map = self.session.piece_map(h);
        let meta = self.session.meta(h);
        let files = self.session.files(h);
        let t = &mut self.torrents[i];
        if !map.is_empty() {
            t.rec.piece_count = map.len() as u64;
            t.rec.bitfield = Some(details::pack_bitfield(&map));
        }
        if meta.has_metadata {
            t.rec.piece_length = meta.piece_length.max(0) as u64;
            let old = t.rec.meta.clone().unwrap_or_default();
            t.rec.meta = Some(TorrentMeta {
                comment: if old.comment.is_empty() { meta.comment.clone() } else { old.comment },
                created_by: if old.created_by.is_empty() { meta.creator.clone() } else { old.created_by },
                created_on: if old.created_on == 0 { meta.created * 1000 } else { old.created_on },
                private: meta.private_flag,
            });
            if !meta.trackers.is_empty() {
                t.rec.announce = Some(meta.trackers.clone());
            }
            if !meta.url_seeds.is_empty() {
                t.rec.web_seeds = Some(meta.url_seeds.clone());
            }
        }
        if !files.is_empty() {
            t.rec.files = Some(
                files
                    .iter()
                    .map(|f| {
                        let size = f.size.max(0) as u64;
                        FileSnap {
                            name: torrent::strip_part(&f.name).to_string(),
                            path: torrent::strip_part(&f.path).to_string(),
                            length: size,
                            downloaded: f.downloaded.max(0) as u64,
                            progress: if size > 0 { f.downloaded.max(0) as f64 / size as f64 } else { 1.0 },
                        }
                    })
                    .collect(),
            );
        }
    }

    fn persist(&mut self) {
        for i in 0..self.torrents.len() {
            self.sync_record(i, false);
        }
        let rows: Vec<TorrentRecord> = self
            .torrents
            .iter()
            .map(|t| {
                let mut r = t.rec.clone();
                r.uploaded_base = t.uploaded();
                r.downloaded_base = t.downloaded();
                r.state = t.state();
                r
            })
            .collect();
        self.store.data.torrents = rows;
        self.store.save();
    }

    fn resume_path(&self, t: &Torrent) -> Option<PathBuf> {
        let hash = t.rec.info_hash.as_deref()?;
        (hash.len() >= 40 && hash.chars().all(|c| c.is_ascii_hexdigit())).then(|| self.resume_dir.join(format!("{hash}.resume")))
    }

    fn write_resume(&self, i: usize, data: &[u8]) {
        let Some(path) = self.resume_path(&self.torrents[i]) else { return };
        let tmp = path.with_extension("resume.tmp");
        let result = std::fs::create_dir_all(&self.resume_dir)
            .and_then(|_| std::fs::write(&tmp, data))
            .and_then(|_| std::fs::rename(&tmp, &path));
        if let Err(err) = result {
            eprintln!("[engine] could not write resume data: {err}");
        }
    }

    // ------------------------------------------------------------- queue

    /// Starts as many wanting-to-run torrents as the queue settings allow.
    fn pump_queue(&mut self) {
        let s = self.store.settings().clone();
        let mut order: Vec<usize> = (0..self.torrents.len()).collect();
        order.sort_by_key(|&i| self.torrents[i].order());

        let mut active_total = 0;
        let mut active_downloads = 0;
        for &i in &order {
            let t = &self.torrents[i];
            if t.handle.is_none() || t.stopping || t.state() == State::Paused {
                continue;
            }
            active_total += 1;
            if t.done() < 1.0 {
                active_downloads += 1;
            }
        }
        for &i in &order {
            let t = &self.torrents[i];
            if t.handle.is_some() || !t.want_start {
                continue;
            }
            let will_seed = t.rec.progress >= 1.0;
            if active_total >= s.max_active_torrents || (!will_seed && active_downloads >= s.max_active_downloads) {
                self.torrents[i].rec.state = State::Queued;
                continue;
            }
            self.spawn_torrent(i);
            active_total += 1;
            if !will_seed {
                active_downloads += 1;
            }
        }
    }

    fn spawn_torrent(&mut self, i: usize) {
        let settings = self.store.settings().clone();
        let t = &self.torrents[i];
        if t.handle.is_some() {
            return;
        }
        let bytes = t.rec.torrent_file.as_deref().and_then(|b| B64.decode(b).ok());
        let magnet = t.rec.magnet_uri.clone().unwrap_or_default();
        if bytes.is_none() && magnet.is_empty() {
            let t = &mut self.torrents[i];
            t.rec.state = State::Error;
            t.error = Some("No torrent data".into());
            return;
        }
        let save_path = PathBuf::from(&t.rec.save_path);
        let mut params = lt::AddParams {
            save_path: t.rec.save_path.clone(),
            sequential: t.rec.sequential,
            paused: self.bind_address == Some(None),
            disable_pex: !self.started.enable_pex,
            strip_udp_trackers: self.policy.is_some(),
            max_uploads: settings.max_upload_slots.clamp(1, i32::MAX as u64) as i32,
            ..Default::default()
        };
        let mut plan = Vec::new();
        if let Some(meta) = bytes.as_deref().and_then(|b| lt::inspect_buffer(b).ok()) {
            plan = plan_parts(&save_path, &meta.files, settings.part_files);
            params.renamed = plan
                .iter()
                .filter(|p| p.part)
                .map(|p| lt::RenamedFile { index: p.index as i32, path: format!("{}{PART_SUFFIX}", p.rel) })
                .collect();
            if !t.rec.priorities.is_empty() {
                params.file_priorities = (0..meta.files.len()).map(|f| lt::lt_priority(t.priority(f))).collect();
            }
        }
        if let Some(path) = self.resume_path(t) {
            params.resume = std::fs::read(path).unwrap_or_default();
        }
        match bytes {
            Some(b) => params.torrent = b,
            None => params.magnet = magnet,
        }

        let name = t.rec.name.clone();
        match self.session.pin_mut().add(&params) {
            Ok(h) => {
                let status = self.session.status(h);
                let id = self.torrents[i].rec.id.clone();
                self.by_handle.insert(h, id.clone());
                let t = &mut self.torrents[i];
                t.handle = Some(h);
                t.plan = plan;
                t.error = None;
                t.stopping = false;
                t.up_at_spawn = None;
                t.down_at_spawn = None;
                t.status = status;
                t.rec.state = State::Checking;
                if params.paused {
                    self.suspended.push(id);
                }
                self.fix_part_names(i);
            }
            Err(err) => {
                let t = &mut self.torrents[i];
                t.rec.state = State::Error;
                t.error = Some(err.what().to_string());
                self.log(format!("Failed to start \"{name}\": {}", err.what()), LogLevel::Error);
            }
        }
    }

    /// Makes the names libtorrent is using match the part plan -- resume data
    /// can carry names from before a file completed, and a magnet gets its plan
    /// only once metadata arrives.
    fn fix_part_names(&mut self, i: usize) {
        let Some(h) = self.torrents[i].handle else { return };
        let files = self.session.files(h);
        if files.is_empty() {
            return;
        }
        if self.torrents[i].plan.is_empty() {
            let t = &self.torrents[i];
            let plan = plan_parts(Path::new(&t.rec.save_path), &files, self.store.settings().part_files);
            self.torrents[i].plan = plan;
        }
        for entry in &self.torrents[i].plan {
            let want = if entry.part { format!("{}{PART_SUFFIX}", entry.rel) } else { entry.rel.clone() };
            if files.get(entry.index).is_some_and(|f| f.path != want) {
                self.session.rename_file(h, entry.index as i32, &want);
            }
        }
    }

    /// Drop the .part suffix now that every wanted piece is present. Files set to
    /// Don't Download stay parked -- they are not complete.
    fn commit_part_files(&mut self, i: usize, first: bool) {
        let Some(h) = self.torrents[i].handle else { return self.finish_complete(i, first) };
        let pending: Vec<(usize, String)> = self.torrents[i]
            .plan
            .iter()
            .filter(|p| p.part && self.torrents[i].priority(p.index) != 0)
            .map(|p| (p.index, p.rel.clone()))
            .collect();
        let t = &mut self.torrents[i];
        t.renamed = 0;
        t.rename_error = None;
        t.pending_renames = pending.len();
        t.status.name.clear();
        if pending.is_empty() {
            return self.finish_complete(i, first);
        }
        self.torrents[i].rec.extra.insert("_notify".into(), serde_json::Value::Bool(first));
        for (index, rel) in pending {
            self.session.rename_file(h, index as i32, &rel);
        }
    }

    fn finish_complete(&mut self, i: usize, first: bool) {
        if let Some(h) = self.torrents[i].handle {
            let status = self.session.status(h);
            self.torrents[i].status = status;
        }
        self.sync_record(i, true);
        let t = &self.torrents[i];
        let name = t.rec.name.clone();
        if let Some(err) = t.rename_error.clone() {
            self.log(format!("Could not rename {PART_SUFFIX} files for \"{name}\": {err}"), LogLevel::Error);
        } else if t.renamed > 0 {
            self.log(format!("Finalised {} file(s) for \"{name}\".", t.renamed), LogLevel::Info);
        }
        if first {
            self.log(format!("\"{name}\" finished downloading."), LogLevel::Info);
            let t = &self.torrents[i];
            self.emit(Event::Complete { id: t.rec.id.clone(), name, path: t.rec.save_path.clone() });
        }
        self.persist();
        self.pump_queue();
    }

    fn enforce_seed_goals(&mut self) {
        let s = self.store.settings();
        let ratio_goal = if s.seed_ratio_limit > 0 { s.seed_ratio_limit as f64 / 100.0 } else { 0.0 };
        let time_goal = if s.seed_time_limit > 0 { s.seed_time_limit as i64 * 60_000 } else { 0 };
        if ratio_goal == 0.0 && time_goal == 0 {
            return;
        }
        let mut hits = Vec::new();
        for t in &self.torrents {
            if !t.live() || t.state() != State::Seeding {
                continue;
            }
            let ratio = share_ratio(t.downloaded(), t.uploaded(), t.length());
            let hit_ratio = ratio_goal > 0.0 && ratio >= ratio_goal;
            let hit_time = time_goal > 0 && t.rec.completed_on > 0 && now_ms() - t.rec.completed_on >= time_goal;
            if hit_ratio {
                hits.push((t.rec.id.clone(), format!("\"{}\" reached its share ratio goal ({ratio:.2}); seeding stopped.", t.rec.name)));
            } else if hit_time {
                hits.push((t.rec.id.clone(), format!("\"{}\" reached its seeding time goal; seeding stopped.", t.rec.name)));
            }
        }
        for (id, msg) in hits {
            self.log(msg, LogLevel::Info);
            self.pause(&id);
        }
    }

    /// The kill switch. The bound interface is looked at every second: when it
    /// goes away, every running torrent is held until it returns, so nothing can
    /// fall back to another route; when it comes back, possibly on a new address,
    /// the sockets are reopened on it and the held torrents resume.
    fn check_interface(&mut self) {
        let Some(name) = self.policy.as_ref().and_then(|p| p.bind.clone()) else { return };
        let addr = net::bind_address(&name).map(|a| a.to_string());
        if self.bind_address.as_ref() == Some(&addr) {
            return;
        }
        let previous = self.bind_address.replace(addr.clone());
        match addr {
            None => {
                self.log(format!("{} -- every connection is held until it returns.", net::interface_gone(&name)), LogLevel::Warn);
                for t in &self.torrents {
                    if let (Some(h), true) = (t.handle, t.live() && !t.status.paused) {
                        self.session.pause(h);
                        self.suspended.push(t.rec.id.clone());
                    }
                }
            }
            Some(a) => {
                self.session.pin_mut().reopen_network_sockets();
                if matches!(previous, Some(None)) {
                    self.log(format!("Interface {name} is back ({a}); resuming."), LogLevel::Info);
                    for id in std::mem::take(&mut self.suspended) {
                        if let Some(t) = self.index(&id).map(|i| &self.torrents[i]) {
                            if let (Some(h), true) = (t.handle, t.want_start) {
                                self.session.resume(h);
                            }
                        }
                    }
                } else if previous.is_some() {
                    self.log(format!("Interface {name} now has address {a}."), LogLevel::Info);
                }
            }
        }
    }

    fn apply_settings(&mut self) {
        let live = self.store.settings().clone();
        let config = config::session_config(&self.started, &live, self.policy.as_ref(), &self.version);
        self.session.pin_mut().apply_config(&config);
        let slots = live.max_upload_slots.clamp(1, i32::MAX as u64) as i32;
        for t in &self.torrents {
            if let Some(h) = t.handle {
                self.session.set_max_uploads(h, slots);
            }
        }
        self.pump_queue();
    }

    fn remember_label(&mut self, label: &str) {
        if label.is_empty() || self.store.data.labels.iter().any(|l| l == label) {
            return;
        }
        self.store.data.labels.push(label.to_string());
        self.store.save();
        self.emit_library();
    }

    fn emit_library(&self) {
        self.emit(Event::LibraryChanged {
            labels: self.store.data.labels.clone(),
            label_styles: self.store.data.label_styles.clone(),
        });
    }

    // ---------------------------------------------------------- commands

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Bootstrap(reply) => {
                let _ = reply.send(Bootstrap {
                    settings: self.store.settings().clone(),
                    labels: self.store.data.labels.clone(),
                    label_styles: self.store.data.label_styles.clone(),
                    log: self.log.iter().cloned().collect(),
                    columns: self.store.data.columns.clone(),
                    window: self.store.data.window.clone(),
                    rows: self.rows(),
                    globals: self.globals(),
                });
            }
            Command::Details(id) => {
                self.selected = id;
                self.send_tick();
            }
            Command::SetSettings(patch) => self.set_settings(patch),
            Command::ToggleAltSpeed => {
                let on = !self.store.settings().alt_speed_enabled;
                self.store.patch_settings(&settings::patch("altSpeedEnabled", on));
                self.apply_settings();
                self.log(if on { "Alternate speed limits enabled." } else { "Alternate speed limits disabled." }, LogLevel::Info);
                self.emit(Event::SettingsChanged(self.store.settings().clone()));
                self.send_tick();
            }
            Command::SetLabelStyle { name, style } => {
                if name.is_empty() {
                    return;
                }
                match style.filter(|s| !s.symbol.is_empty() || !s.color.is_empty()) {
                    Some(s) => {
                        self.store.data.label_styles.insert(name, s);
                    }
                    // Back to the default: deleted, so the file does not collect
                    // entries for labels that were only ever looked at.
                    None => {
                        self.store.data.label_styles.shift_remove(&name);
                    }
                }
                self.store.save();
                self.emit_library();
            }
            Command::SetColumns(cols) => {
                self.store.data.columns = Some(cols);
                self.store.save();
            }
            Command::SetWindow(bounds) => {
                self.store.data.window = Some(bounds);
                self.store.save();
            }
            Command::Interfaces(reply) => {
                let _ = reply.send(net::list_interfaces());
            }
            Command::Inspect { source, reply } => self.inspect(source, reply),
            Command::Add { source, options, reply } => self.add(source, options, reply),
            Command::AddPaths(paths) => {
                let save_path = Some(self.store.settings().last_save_path.clone()).filter(|p| !p.is_empty());
                for path in paths {
                    let options = AddOptions { save_path: save_path.clone(), ..Default::default() };
                    self.add(TorrentSource::Path(path), options, None);
                }
            }
            Command::CreateTorrent { options, progress, reply } => self.create(options, progress, reply),
            Command::SaveTorrentFile { id, dest, reply } => {
                let result = match self.index(&id).and_then(|i| self.torrents[i].rec.torrent_file.clone()) {
                    None => Err("Torrent metadata is not available yet.".to_string()),
                    Some(b64) => B64
                        .decode(b64)
                        .map_err(|e| e.to_string())
                        .and_then(|bytes| std::fs::write(&dest, bytes).map_err(|e| e.to_string())),
                };
                if result.is_ok() {
                    let name = self.index(&id).map(|i| self.torrents[i].rec.name.clone()).unwrap_or_default();
                    self.log(format!("Saved \"{name}\" as {}", dest.display()), LogLevel::Info);
                }
                let _ = reply.send(result);
            }
            Command::Start(ids) => ids.iter().for_each(|id| self.start_torrent(id)),
            Command::Pause(ids) => ids.iter().for_each(|id| self.pause(id)),
            Command::Stop(ids) => ids.iter().for_each(|id| self.stop(id)),
            Command::Recheck(ids) => ids.iter().for_each(|id| self.recheck(id)),
            Command::Remove { ids, delete_data } => {
                for id in &ids {
                    self.remove(id, delete_data);
                }
                self.persist();
                self.pump_queue();
                self.send_tick();
            }
            Command::MoveQueue { id, delta } => {
                let mut order: Vec<usize> = (0..self.torrents.len()).collect();
                order.sort_by_key(|&i| self.torrents[i].order());
                let Some(pos) = order.iter().position(|&i| self.torrents[i].id() == id) else { return };
                let target = pos as i64 + delta as i64;
                if target < 0 || target >= order.len() as i64 {
                    return;
                }
                let (a, b) = (order[pos], order[target as usize]);
                let (oa, ob) = (self.torrents[a].rec.order, self.torrents[b].rec.order);
                self.torrents[a].rec.order = ob;
                self.torrents[b].rec.order = oa;
                self.persist();
                self.send_tick();
            }
            Command::SetLabel { ids, label } => {
                for id in &ids {
                    if let Some(i) = self.index(id) {
                        self.torrents[i].rec.label = label.clone();
                    }
                }
                self.remember_label(&label);
                self.persist();
                self.send_tick();
            }
            Command::SetSequential { id, on } => {
                let Some(i) = self.index(&id) else { return };
                self.torrents[i].rec.sequential = on;
                if let Some(h) = self.torrents[i].handle {
                    self.session.set_sequential(h, on);
                }
                self.persist();
                self.send_tick();
            }
            Command::SetFilePriority { id, index, priority } => {
                let Some(i) = self.index(&id) else { return };
                let priority = priority.min(2);
                let t = &mut self.torrents[i];
                t.rec.priorities.insert(index, priority);
                if let Some(count) = t.rec.files.as_ref().map(|f| f.len()) {
                    t.rec.wanted = Some((0..count).filter(|f| t.priority(*f) != 0).collect());
                }
                if let Some(h) = t.handle {
                    self.session.set_file_priority(h, index as i32, lt::lt_priority(priority));
                }
                self.persist();
                self.send_tick();
            }
            Command::AddTracker { id, url } => {
                let Some(i) = self.index(&id) else { return };
                let name = self.torrents[i].rec.name.clone();
                if self.policy.is_some() && egress::is_udp_tracker(&url) {
                    self.log(
                        format!("Not adding {url}: udp:// trackers cannot be routed while a proxy or interface binding is on."),
                        LogLevel::Warn,
                    );
                    return;
                }
                if let Some(h) = self.torrents[i].handle {
                    if self.session.add_tracker(h, &url) {
                        self.log(format!("Added tracker {url} to \"{name}\"."), LogLevel::Info);
                    }
                }
            }
            Command::AddPeer { id, address } => {
                let Some(h) = self.index(&id).and_then(|i| self.torrents[i].handle) else { return };
                let ok = parse_peer(&address).is_some_and(|(ip, port)| self.session.connect_peer(h, &ip, port).is_ok());
                if ok {
                    self.log(format!("Added peer {address}."), LogLevel::Info);
                } else {
                    self.log(format!("Peer {address} rejected."), LogLevel::Warn);
                }
            }
            Command::Reannounce(id) => {
                let Some(i) = self.index(&id) else { return };
                if let Some(h) = self.torrents[i].handle {
                    self.session.reannounce(h);
                    let name = self.torrents[i].rec.name.clone();
                    self.log(format!("Re-announced \"{name}\" to its trackers."), LogLevel::Info);
                }
            }
            Command::ResolvePath { id, file, reply } => {
                let _ = reply.send(self.index(&id).and_then(|i| self.resolve_path(i, file)));
            }
            Command::Shutdown(reply) => {
                self.shutdown();
                let _ = reply.send(());
            }
        }
    }

    fn set_settings(&mut self, mut patch: settings::SettingsPatch) {
        settings::normalise_patch(self.store.settings(), &mut patch);
        // Read before the patch lands, so the user is told their proxy is not in
        // force yet rather than left to assume it is.
        let next_policy = EgressPolicy::from_settings(&self.store.settings().patched(&patch));
        let moved = next_policy != self.policy;
        let next = self.store.patch_settings(&patch).clone();
        if moved {
            self.log(
                if next.proxy_enabled || !next.bind_interface.is_empty() {
                    "Proxy or interface binding changed. It takes effect when ztorrent restarts -- until then traffic goes out as it did before."
                } else {
                    "Proxy and interface binding turned off. They stay in force until ztorrent restarts."
                },
                LogLevel::Info,
            );
        }
        self.apply_settings();
        self.emit(Event::SettingsChanged(next));
        self.send_tick();
    }

    fn start_torrent(&mut self, id: &str) {
        let Some(i) = self.index(id) else { return };
        let t = &mut self.torrents[i];
        t.want_start = true;
        let name = t.rec.name.clone();
        match t.handle {
            Some(h) if !t.stopping => {
                self.session.resume(h);
                self.suspended.retain(|s| s != id);
                let t = &mut self.torrents[i];
                t.status = self.session.status(h);
                t.rec.state = t.state();
            }
            _ => {
                t.rec.state = State::Queued;
                self.pump_queue();
            }
        }
        self.log(format!("Started \"{name}\"."), LogLevel::Info);
        self.persist();
        self.send_tick();
    }

    fn pause(&mut self, id: &str) {
        let Some(i) = self.index(id) else { return };
        let Some(h) = self.torrents[i].handle.filter(|_| !self.torrents[i].stopping) else { return };
        // libtorrent's pause stops the transfer outright; the WebTorrent code needed
        // to clear piece selections by hand for the same effect.
        self.session.pause(h);
        let t = &mut self.torrents[i];
        t.status = self.session.status(h);
        t.rec.state = State::Paused;
        t.want_start = false;
        let name = t.rec.name.clone();
        self.log(format!("Paused \"{name}\"."), LogLevel::Info);
        self.persist();
        self.send_tick();
    }

    /// Stop tears the swarm down entirely but keeps the torrent in the list.
    fn stop(&mut self, id: &str) {
        let Some(i) = self.index(id) else { return };
        self.torrents[i].want_start = false;
        if let Some(h) = self.torrents[i].handle.filter(|_| !self.torrents[i].stopping) {
            if let Some(t) = self.torrents.get_mut(i) {
                t.status = self.session.status(h);
            }
            self.sync_record(i, true);
            let t = &mut self.torrents[i];
            t.rec.uploaded_base = t.uploaded();
            t.rec.downloaded_base = t.downloaded();
            t.rec.progress = t.done();
            t.stopping = true;
            if !self.session.save_resume_data(h) {
                self.complete_stop(i);
            }
        }
        let t = &mut self.torrents[i];
        t.rec.state = State::Stopped;
        let name = t.rec.name.clone();
        self.log(format!("Stopped \"{name}\"."), LogLevel::Info);
        self.persist();
        self.pump_queue();
        self.send_tick();
    }

    fn complete_stop(&mut self, i: usize) {
        let t = &mut self.torrents[i];
        if let Some(h) = t.handle.take() {
            self.session.pin_mut().remove(h, false);
            self.by_handle.remove(&h);
        }
        let t = &mut self.torrents[i];
        t.stopping = false;
        t.status = lt::Status::default();
        t.up_at_spawn = None;
        t.down_at_spawn = None;
        self.pump_queue();
    }

    fn recheck(&mut self, id: &str) {
        let Some(i) = self.index(id) else { return };
        match self.torrents[i].handle.filter(|_| !self.torrents[i].stopping) {
            Some(h) => {
                self.torrents[i].rechecking = true;
                self.session.recheck(h);
                let name = self.torrents[i].rec.name.clone();
                self.log(format!("Re-checking \"{name}\"..."), LogLevel::Info);
                self.send_tick();
            }
            // Stopped torrents are re-checked by starting them: every piece on disk
            // is verified as the session takes them on.
            None => self.start_torrent(id),
        }
    }

    fn remove(&mut self, id: &str, delete_data: bool) {
        let Some(i) = self.index(id) else { return };
        let name = self.torrents[i].rec.name.clone();
        let save_path = PathBuf::from(&self.torrents[i].rec.save_path);
        // Whether libtorrent took the job. It only did if it still had the torrent.
        let released = match self.torrents[i].handle.take() {
            Some(h) => {
                self.by_handle.remove(&h);
                self.session.pin_mut().remove(h, delete_data)
            }
            None => false,
        };
        if delete_data && !released && name != "Downloading metadata" {
            match paths::safe_top_level(&save_path, &name) {
                Some(target) => {
                    let part = PathBuf::from(format!("{}{PART_SUFFIX}", target.display()));
                    for path in [target, part] {
                        let result = if path.is_dir() {
                            std::fs::remove_dir_all(&path)
                        } else if path.exists() {
                            std::fs::remove_file(&path)
                        } else {
                            Ok(())
                        };
                        if let Err(err) = result {
                            self.log(format!("Could not delete data for \"{name}\": {err}"), LogLevel::Error);
                        }
                    }
                }
                None => self.log(
                    format!("Refusing to delete data for \"{name}\": that name is not a plain file or folder name."),
                    LogLevel::Error,
                ),
            }
        }
        if let Some(path) = self.resume_path(&self.torrents[i]) {
            let _ = std::fs::remove_file(path);
        }
        self.torrents.remove(i);
        if self.selected.as_deref() == Some(id) {
            self.selected = None;
        }
        self.log(
            if delete_data { format!("Removed \"{name}\" and deleted its data.") } else { format!("Removed \"{name}\".") },
            LogLevel::Info,
        );
    }

    fn resolve_path(&self, i: usize, file: Option<usize>) -> Option<ResolvedPath> {
        let t = &self.torrents[i];
        let save_path = PathBuf::from(&t.rec.save_path);
        let target = match file {
            Some(index) => {
                let live = t.handle.map(|h| self.session.files(h)).and_then(|f| f.get(index).map(|f| f.path.clone()));
                let rel = live.or_else(|| t.rec.files.as_ref()?.get(index).map(|f| f.path.clone()))?;
                paths::safe_join(&save_path, &rel)?
            }
            None => {
                let top = paths::safe_top_level(&save_path, &t.rec.name)?;
                if top.exists() {
                    top
                } else {
                    PathBuf::from(format!("{}{PART_SUFFIX}", top.display()))
                }
            }
        };
        Some(ResolvedPath { exists: target.exists(), target, save_path })
    }

    // -------------------------------------------------------------- adding

    fn inspect(&mut self, source: TorrentSource, reply: Reply<Result<InspectInfo, String>>) {
        let policy = self.policy.clone();
        let version = self.version.clone();
        std::thread::spawn(move || {
            let meta = match source {
                TorrentSource::Magnet(m) => lt::inspect_magnet(&m).map_err(|e| e.what().to_string()),
                TorrentSource::Url(u) => net::fetch_torrent(&u, policy.as_ref(), &version)
                    .and_then(|b| lt::inspect_buffer(&b).map_err(|e| e.what().to_string())),
                TorrentSource::Path(p) => read_torrent_file(&p).and_then(|b| lt::inspect_buffer(&b).map_err(|e| e.what().to_string())),
                TorrentSource::Bytes(b) => lt::inspect_buffer(&b).map_err(|e| e.what().to_string()),
            };
            let _ = reply.send(meta.map(|m| InspectInfo {
                name: if m.name.is_empty() { m.info_hash.clone() } else { m.name.clone() },
                info_hash: m.info_hash.clone(),
                length: m.total_size.max(0) as u64,
                piece_length: m.piece_length.max(0) as u64,
                comment: m.comment.clone(),
                created_by: m.creator.clone(),
                created: m.created * 1000,
                private: m.private_flag,
                announce: m.trackers.clone(),
                files: m
                    .files
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| !f.pad)
                    .map(|(index, f)| InspectFile { index, name: f.name.clone(), path: f.path.clone(), length: f.size.max(0) as u64 })
                    .collect(),
            }));
        });
    }

    fn add(&mut self, source: TorrentSource, options: AddOptions, reply: Option<Reply<Result<AddOutcome, String>>>) {
        match source {
            TorrentSource::Url(url) => {
                let policy = self.policy.clone();
                let version = self.version.clone();
                let tx = self.internal_tx.clone();
                std::thread::spawn(move || {
                    let bytes = net::fetch_torrent(&url, policy.as_ref(), &version);
                    let _ = tx.send(Internal::Fetched { bytes, options, reply });
                });
            }
            TorrentSource::Path(path) => match read_torrent_file(&path) {
                Ok(bytes) => self.add_parsed(Some(bytes), None, options, reply),
                Err(err) => self.add_failed(err, reply),
            },
            TorrentSource::Bytes(bytes) => self.add_parsed(Some(bytes), None, options, reply),
            TorrentSource::Magnet(m) => self.add_parsed(None, Some(m), options, reply),
        }
    }

    fn add_failed(&mut self, err: String, reply: Option<Reply<Result<AddOutcome, String>>>) {
        self.log(format!("Could not read torrent: {err}"), LogLevel::Error);
        if let Some(r) = reply {
            let _ = r.send(Err(err));
        }
    }

    fn add_parsed(&mut self, bytes: Option<Vec<u8>>, magnet: Option<String>, options: AddOptions, reply: Option<Reply<Result<AddOutcome, String>>>) {
        let parsed = match (&bytes, &magnet) {
            (Some(b), _) => lt::inspect_buffer(b).map_err(|e| e.what().to_string()),
            (None, Some(m)) => lt::inspect_magnet(m).map_err(|e| e.what().to_string()),
            _ => Err("Unrecognised torrent source".to_string()),
        };
        let meta = match parsed {
            Ok(m) => m,
            Err(err) => return self.add_failed(err, reply),
        };
        let info_hash = meta.info_hash.to_ascii_lowercase();
        if let Some(existing) = self.torrents.iter().find(|t| t.rec.info_hash.as_deref() == Some(info_hash.as_str())) {
            let (id, name) = (existing.rec.id.clone(), existing.rec.name.clone());
            self.log(format!("\"{name}\" is already in the list."), LogLevel::Warn);
            if let Some(r) = reply {
                let _ = r.send(Ok(AddOutcome::Duplicate(id)));
            }
            return;
        }

        let settings = self.store.settings().clone();
        let order = self.order_seq;
        self.order_seq += 1;
        let files = (!meta.files.is_empty()).then(|| {
            meta.files
                .iter()
                .map(|f| FileSnap { name: f.name.clone(), path: f.path.clone(), length: f.size.max(0) as u64, downloaded: 0, progress: 0.0 })
                .collect()
        });
        let rec = TorrentRecord {
            id: uuid::Uuid::new_v4().to_string(),
            info_hash: Some(info_hash.clone()),
            name: if meta.name.is_empty() { "Downloading metadata".into() } else { meta.name.clone() },
            magnet_uri: magnet.clone().or_else(|| Some(format!("magnet:?xt=urn:btih:{info_hash}"))),
            torrent_file: bytes.as_ref().map(|b| B64.encode(b)),
            save_path: options.save_path.clone().filter(|p| !p.is_empty()).unwrap_or(settings.download_path.clone()),
            label: options.label.clone(),
            order: Some(order),
            added_on: now_ms(),
            length: meta.total_size.max(0) as u64,
            wanted: options.wanted.clone(),
            priorities: options.priorities.clone(),
            sequential: options.sequential.unwrap_or(settings.sequential_download),
            piece_length: meta.piece_length.max(0) as u64,
            files,
            announce: (!meta.trackers.is_empty()).then(|| meta.trackers.clone()),
            web_seeds: (!meta.url_seeds.is_empty()).then(|| meta.url_seeds.clone()),
            meta: bytes.as_ref().map(|_| TorrentMeta {
                comment: meta.comment.clone(),
                created_by: meta.creator.clone(),
                created_on: meta.created * 1000,
                private: meta.private_flag,
            }),
            state: State::Queued,
            ..Default::default()
        };
        let id = rec.id.clone();
        let name = rec.name.clone();
        let mut t = Torrent::new(rec);
        t.want_start = !options.paused && settings.start_torrents_automatically;
        let want = t.want_start;
        self.torrents.push(t);
        self.remember_label(&options.label);
        self.log(format!("Added \"{name}\"."), LogLevel::Info);
        if want {
            self.pump_queue();
        } else if let Some(i) = self.index(&id) {
            self.torrents[i].rec.state = State::Paused;
        }
        self.persist();
        self.send_tick();
        if let Some(r) = reply {
            let _ = r.send(Ok(AddOutcome::Added(id)));
        }
    }

    fn create(&mut self, options: CreateTorrentOptions, progress: Option<futures_channel::mpsc::UnboundedSender<f32>>, reply: Reply<Result<u64, String>>) {
        let creator = format!("ztorrent {}", self.version);
        let tx = self.internal_tx.clone();
        std::thread::spawn(move || {
            let mut trackers = Vec::new();
            let mut tiers = Vec::new();
            for (tier, group) in options.trackers.iter().enumerate() {
                for url in group.iter().map(|u| u.trim()).filter(|u| !u.is_empty()) {
                    trackers.push(url.to_string());
                    tiers.push(tier as i32);
                }
            }
            let params = lt::CreateParams {
                input: options.input_path.to_string_lossy().into_owned(),
                trackers,
                tiers,
                web_seeds: options.web_seeds.iter().map(|w| w.trim().to_string()).filter(|w| !w.is_empty()).collect(),
                comment: options.comment.clone(),
                creator,
                private_flag: options.private,
                piece_size: options.piece_length as i32,
            };
            let sink = lt::ProgressSink(Box::new(move |done, total| {
                if let Some(p) = &progress {
                    let _ = p.unbounded_send(if total > 0 { done as f32 / total as f32 } else { 0.0 });
                }
            }));
            let result = lt::create_torrent(&params, &sink)
                .map_err(|e| e.what().to_string())
                .and_then(|bytes| std::fs::write(&options.output_path, &bytes).map(|_| bytes).map_err(|e| e.to_string()));
            match result {
                Ok(bytes) => {
                    let _ = reply.send(Ok(bytes.len() as u64));
                    let _ = tx.send(Internal::Created { options, bytes });
                }
                Err(err) => {
                    let _ = tx.send(Internal::Log(format!("Could not create torrent: {err}"), LogLevel::Error));
                    let _ = reply.send(Err(err));
                }
            }
        });
    }

    fn handle_internal(&mut self, msg: Internal) {
        match msg {
            Internal::Log(message, level) => self.log(message, level),
            Internal::Fetched { bytes, options, reply } => match bytes {
                Ok(b) => self.add_parsed(Some(b), None, options, reply),
                Err(err) => self.add_failed(err, reply),
            },
            Internal::Created { options, bytes } => {
                let name = options.input_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.log(format!("Created torrent \"{name}\" -> {}", options.output_path.display()), LogLevel::Info);
                if options.start_seeding {
                    let save_path = options.input_path.parent().map(|p| p.to_string_lossy().into_owned());
                    let opts = AddOptions { save_path, paused: false, ..Default::default() };
                    self.add_parsed(Some(bytes), None, opts, None);
                }
            }
        }
    }

    // --------------------------------------------------------------- alerts

    fn drain_alerts(&mut self) {
        for alert in self.session.pin_mut().pop_alerts() {
            let i = if alert.handle != 0 { self.index_of_handle(alert.handle) } else { None };
            use lt::AlertKind as K;
            match (alert.kind, i) {
                (K::MetadataReceived, Some(i)) => self.on_metadata(i),
                (K::TorrentFinished, Some(i)) => {
                    let t = &mut self.torrents[i];
                    let first = t.rec.completed_on == 0;
                    if first {
                        t.rec.completed_on = now_ms();
                    }
                    t.rec.state = State::Seeding;
                    self.commit_part_files(i, first);
                }
                (K::TorrentError | K::FileError, Some(i)) => {
                    let name = self.torrents[i].rec.name.clone();
                    self.torrents[i].error = Some(alert.message.clone());
                    self.log(format!("Error on \"{name}\": {}", alert.message), LogLevel::Error);
                }
                (K::ResumeData, Some(i)) => {
                    self.write_resume(i, &alert.data);
                    if self.torrents[i].stopping {
                        self.complete_stop(i);
                        self.persist();
                    }
                }
                (K::ResumeDataFailed, Some(i)) => {
                    if self.torrents[i].stopping {
                        self.complete_stop(i);
                        self.persist();
                    }
                }
                (K::FileRenamed, Some(i)) => {
                    if alert.message.ends_with(PART_SUFFIX) {
                        continue;
                    }
                    let index = alert.index.max(0) as usize;
                    let t = &mut self.torrents[i];
                    if let Some(entry) = t.plan.iter_mut().find(|p| p.index == index) {
                        entry.part = false;
                    }
                    if t.pending_renames > 0 {
                        t.renamed += 1;
                        t.pending_renames -= 1;
                        if t.pending_renames == 0 {
                            let first = t.rec.extra.remove("_notify").and_then(|v| v.as_bool()).unwrap_or(false);
                            self.finish_complete(i, first);
                        }
                    }
                }
                (K::FileRenameFailed, Some(i)) => {
                    let index = alert.index.max(0) as usize;
                    let t = &mut self.torrents[i];
                    if t.pending_renames > 0 {
                        t.pending_renames -= 1;
                        // A zero-length or never-written file has nothing to move.
                        if alert.message.contains("No such file") {
                            if let Some(entry) = t.plan.iter_mut().find(|p| p.index == index) {
                                entry.part = false;
                            }
                        } else {
                            t.rename_error = Some(alert.message.clone());
                        }
                        if t.pending_renames == 0 {
                            let first = t.rec.extra.remove("_notify").and_then(|v| v.as_bool()).unwrap_or(false);
                            self.finish_complete(i, first);
                        }
                    }
                }
                (K::TorrentChecked, Some(i)) => {
                    if std::mem::take(&mut self.torrents[i].rechecking) {
                        if let Some(h) = self.torrents[i].handle {
                            self.torrents[i].status = self.session.status(h);
                        }
                        let (name, done) = (self.torrents[i].rec.name.clone(), self.torrents[i].done());
                        self.log(format!("Re-check of \"{name}\" complete ({:.1}%).", done * 100.0), LogLevel::Info);
                    }
                }
                (K::ListenSucceeded, _) => {
                    if std::env::var_os("ZTORRENT_DEBUG_LISTEN").is_some() {
                        eprintln!("[listen] {}:{} socket_type={}", alert.message, alert.number, alert.index);
                    }
                    // socket_type 0 is TCP, 6 TCP over SSL: the port peers connect to.
                    // A uTP socket libtorrent keeps for outgoing use is not a listener.
                    if self.listen_port == 0 && matches!(alert.index, 0 | 6) {
                        self.listen_port = alert.number.clamp(0, 65535) as u16;
                    }
                }
                (K::ListenFailed, _) => self.log(format!("Could not open a listening socket: {}", alert.message), LogLevel::Warn),
                (K::DhtBootstrap, _) => self.log("DHT bootstrap complete.", LogLevel::Info),
                (K::TorrentDeleteFailed, _) => self.log(format!("Could not delete data: {}", alert.message), LogLevel::Error),
                _ => {}
            }
        }
    }

    fn on_metadata(&mut self, i: usize) {
        let Some(h) = self.torrents[i].handle else { return };
        let meta = self.session.meta(h);
        let bytes = self.session.torrent_bytes(h);
        let t = &mut self.torrents[i];
        if !meta.name.is_empty() {
            t.rec.name = meta.name.clone();
        }
        t.rec.length = meta.total_size.max(0) as u64;
        t.rec.piece_length = meta.piece_length.max(0) as u64;
        t.rec.piece_count = meta.num_pieces.max(0) as u64;
        if !bytes.is_empty() {
            t.rec.torrent_file = Some(B64.encode(&bytes));
        }
        if t.rec.info_hash.is_none() && !meta.info_hash.is_empty() {
            t.rec.info_hash = Some(meta.info_hash.clone());
        }
        let explicit: Vec<(usize, u8)> = t.rec.priorities.iter().map(|(k, v)| (*k, *v)).collect();
        for (index, p) in &explicit {
            self.session.set_file_priority(h, *index as i32, lt::lt_priority(*p));
        }
        self.fix_part_names(i);
        self.sync_record(i, true);
        let t = &mut self.torrents[i];
        if !explicit.is_empty() {
            if let Some(count) = t.rec.files.as_ref().map(|f| f.len()) {
                t.rec.wanted = Some((0..count).filter(|f| t.priority(*f) != 0).collect());
            }
        }
        let name = t.rec.name.clone();
        self.log(format!("Received metadata for \"{name}\"."), LogLevel::Info);
        self.persist();
    }

    fn shutdown(&mut self) {
        self.refresh_status();
        for i in 0..self.torrents.len() {
            self.sync_record(i, true);
        }
        self.persist();
        let mut waiting: HashMap<u64, usize> = HashMap::new();
        for (i, t) in self.torrents.iter().enumerate() {
            if let (Some(h), true) = (t.handle, t.handle.is_some()) {
                if self.session.save_resume_data(h) {
                    waiting.insert(h, i);
                }
            }
        }
        let deadline = Instant::now() + SHUTDOWN_GRACE;
        while !waiting.is_empty() && Instant::now() < deadline {
            if !self.session.pin_mut().wait_for_alert(100) {
                continue;
            }
            for alert in self.session.pin_mut().pop_alerts() {
                match alert.kind {
                    lt::AlertKind::ResumeData => {
                        if let Some(i) = waiting.remove(&alert.handle) {
                            self.write_resume(i, &alert.data);
                        }
                    }
                    lt::AlertKind::ResumeDataFailed => {
                        waiting.remove(&alert.handle);
                    }
                    _ => {}
                }
            }
        }
        self.store.flush();
    }
}

fn read_torrent_file(path: &Path) -> Result<Vec<u8>, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > net::MAX_TORRENT_BYTES {
        return Err("the file is too large to be a torrent".into());
    }
    std::fs::read(path).map_err(|e| e.to_string())
}

/// `1.2.3.4:6881` or `[::1]:6881`.
fn parse_peer(addr: &str) -> Option<(String, u16)> {
    let addr = addr.trim();
    if let Some(rest) = addr.strip_prefix('[') {
        let (ip, port) = rest.split_once("]:")?;
        return Some((ip.to_string(), port.parse().ok()?));
    }
    let (ip, port) = addr.rsplit_once(':')?;
    Some((ip.to_string(), port.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_addresses() {
        assert_eq!(parse_peer("1.2.3.4:6881"), Some(("1.2.3.4".into(), 6881)));
        assert_eq!(parse_peer("[::1]:51413"), Some(("::1".into(), 51413)));
        assert_eq!(parse_peer("nope"), None);
    }
}
