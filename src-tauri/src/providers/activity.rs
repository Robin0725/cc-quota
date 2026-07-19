//! Which provider the user is working with right now.
//!
//! The frontmost application cannot answer this: Codex, Claude and Kimi are routinely all run from
//! the same terminal window, so the foreground app is the terminal in every case. What does
//! separate them is that each CLI leaves a trace when the user submits a prompt — its
//! prompt-history file — so the provider whose history moved most recently is the one the user is
//! working with. Session directories were the original signal and turned out to be wrong: an
//! agent left running unattended writes its session continuously and pins the widget to itself.
//! A root may therefore be a single file, not just a directory. See §6 of
//! `docs/provider-registry-contract.md`.
//!
//! Two rules govern this module:
//!
//! * **Never open a session file.** They contain the user's conversations. Only path modification
//!   times are read, which `fs::metadata` obtains without opening anything.
//! * **Never emit a path.** Session paths embed project names, so nothing here reaches a log, an
//!   error message or a returned value — the only thing that escapes is a provider id.
//!
//! Nothing in this file may branch on a provider id: the directories come from
//! [`ProviderAdapter::activity_paths`], so a fourth provider stays one new file plus one
//! registration line.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::SystemTime,
};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

/// A provider id paired with a directory its CLI writes to.
type Root = (&'static str, PathBuf);

/// Last observed write time per provider.
type Observations = Mutex<HashMap<&'static str, SystemTime>>;

/// How far the fallback descends looking for the newest session file. Session directories are
/// date- or project-partitioned and shallow; the bound only exists so a symlink loop cannot spin.
const MAX_DESCENT_DEPTH: usize = 8;

fn registered_roots() -> Vec<Root> {
    super::all()
        .iter()
        .flat_map(|adapter| {
            let id = adapter.descriptor().id;
            adapter
                .activity_paths()
                .into_iter()
                .map(move |path| (id, path))
        })
        .collect()
}

fn modified(path: &Path) -> Option<SystemTime> {
    // `metadata` stats the path. It never opens it, which is what keeps conversation contents out
    // of this process.
    fs::metadata(path).ok()?.modified().ok()
}

fn record(state: &Observations, provider: &'static str, at: SystemTime) {
    if let Ok(mut observations) = state.lock() {
        let slot = observations.entry(provider).or_insert(at);
        if at > *slot {
            *slot = at;
        }
    }
}

/// Which provider a changed path belongs to, or `None` for anything outside every session
/// directory. Matching on the registered root (not on the watched directory) is what lets the
/// watcher subscribe to a parent directory without mis-attributing its other contents.
fn attribute(roots: &[Root], path: &Path) -> Option<&'static str> {
    roots
        .iter()
        .find(|(_, root)| path.starts_with(root))
        .map(|(provider, _)| *provider)
}

/// The active provider: whoever wrote most recently and still has a local login.
///
/// A provider that is signed out is skipped rather than winning with a stale timestamp, because
/// the widget has nothing to show for it. Returns `None` when nothing has been observed yet, which
/// is the caller's cue to fall back to foreground-app attribution.
pub(crate) fn select(
    observed: &[(&'static str, SystemTime)],
    configured: &[&str],
) -> Option<&'static str> {
    let mut ranked = observed.to_vec();
    ranked.sort_by_key(|(_, at)| std::cmp::Reverse(*at));
    ranked
        .into_iter()
        .find(|(provider, _)| configured.contains(provider))
        .map(|(provider, _)| provider)
}

/// Normalises a root so that reported paths can be matched against it.
///
/// FSEvents reports fully resolved paths, so a session directory reached through a symlink — the
/// `/var` → `/private/var` link macOS ships with, or a home directory relocated to another volume
/// — would never match its own root and every write would go unattributed. Resolving the deepest
/// ancestor that exists and re-attaching the remainder normalises the root without requiring the
/// directory itself to exist yet.
fn resolve(path: &Path) -> PathBuf {
    let mut suffix = Vec::new();
    let mut current = path;
    loop {
        if let Ok(resolved) = current.canonicalize() {
            let mut result = resolved;
            result.extend(suffix.iter().rev());
            return result;
        }
        let (Some(parent), Some(name)) = (current.parent(), current.file_name()) else {
            return path.to_path_buf();
        };
        suffix.push(name.to_owned());
        current = parent;
    }
}

/// Subscribes to every session directory through FSEvents.
///
/// Deliberately *not* a periodic directory scan: one full pass over the session tree measured 25ms
/// on a machine with ~2800 session files, and that cost grows with every session ever recorded. An
/// event subscription costs nothing while the directories are idle and does not care how much
/// history they hold.
/// Returns the watcher (when at least one root could be subscribed) together with the roots that
/// could **not** be — so the caller can poll exactly those instead of silently dropping them. One
/// unwatchable root (its whole parent chain missing at launch, say) must not cost the others
/// their event subscription, and must not itself go dark until a restart.
fn start_watching(
    roots: Vec<Root>,
    state: Arc<Observations>,
) -> (Option<RecommendedWatcher>, Vec<Root>) {
    // Everything downstream works on resolved paths: FSEvents reports fully resolved paths, so
    // both the subscription target and the attribution prefix must live on the same side of any
    // symlink (`/var` → `/private/var`, a prompt-history file symlinked to a synced volume).
    let resolved: Vec<Root> = roots
        .iter()
        .map(|(provider, root)| (*provider, resolve(root)))
        .collect();
    let attribution = resolved.clone();
    let watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
        // Watch errors are dropped rather than logged: the payload carries the offending paths.
        let Ok(event) = result else { return };
        let now = SystemTime::now();
        for path in &event.paths {
            if let Some(provider) = attribute(&attribution, path) {
                record(&state, provider, now);
            }
        }
    });
    let Ok(mut watcher) = watcher else {
        return (None, resolved);
    };

    let mut unwatched = Vec::new();
    for (provider, root) in &resolved {
        // A CLI that has never run has no session directory yet, and one can equally be deleted
        // while the app is running. Subscribing to the parent covers both: the directory's
        // creation is itself an event, and `attribute` keeps the parent's other contents out.
        let target = if root.is_dir() {
            Some(root.clone())
        } else {
            root.parent()
                .filter(|path| path.is_dir())
                .map(Path::to_path_buf)
        };
        let subscribed = match target {
            Some(target) => watcher.watch(&target, RecursiveMode::Recursive).is_ok(),
            None => false,
        };
        if !subscribed {
            unwatched.push((*provider, root.clone()));
        }
    }
    if unwatched.len() == resolved.len() {
        (None, unwatched)
    } else {
        (Some(watcher), unwatched)
    }
}

/// What runs when FSEvents is unavailable.
///
/// Also not a directory scan. Each entry remembers the single newest session file it found and
/// thereafter only stats that file and its directory — two `stat` calls per provider, microseconds
/// each. A rediscovery walk happens only when the remembered directory's own timestamp moves,
/// i.e. only when something was actually written.
struct FallbackEntry {
    provider: &'static str,
    root: PathBuf,
    file: Option<PathBuf>,
    directory: PathBuf,
    directory_stamp: Option<SystemTime>,
    discovered: bool,
}

impl FallbackEntry {
    fn new(provider: &'static str, root: PathBuf) -> Self {
        Self {
            provider,
            directory: root.clone(),
            root,
            file: None,
            directory_stamp: None,
            discovered: false,
        }
    }

    fn rediscover(&mut self) {
        // A root may be a single prompt-history file rather than a tree; it is then its own
        // "newest file" and there is nothing to descend into.
        if self.root.is_file() {
            self.directory = self.root.clone();
            self.file = Some(self.root.clone());
            self.directory_stamp = modified(&self.directory);
            self.discovered = true;
            return;
        }
        match newest_file(&self.root) {
            Some((file, directory)) => {
                self.directory = directory;
                self.file = Some(file);
            }
            None => {
                self.directory = self.root.clone();
                self.file = None;
            }
        }
        self.directory_stamp = modified(&self.directory);
        self.discovered = true;
    }

    fn poll(&mut self, state: &Observations) {
        let moved = match self.directory_stamp {
            Some(stamp) => modified(&self.directory).is_some_and(|now| now > stamp),
            None => true,
        };
        if !self.discovered || moved {
            self.rediscover();
        }
        if let Some(file) = &self.file {
            if let Some(stamp) = modified(file) {
                record(state, self.provider, stamp);
            }
        }
    }
}

/// Finds the newest session file by descending into the most recently touched subdirectory at each
/// level, rather than enumerating the whole tree. Session directories are partitioned by date or
/// by project, so the freshest branch holds the freshest file. Picking the wrong branch only costs
/// the fallback a little accuracy, which is the right trade against an unbounded walk.
fn newest_file(root: &Path) -> Option<(PathBuf, PathBuf)> {
    let mut current = root.to_path_buf();
    for _ in 0..MAX_DESCENT_DEPTH {
        let mut newest_leaf: Option<(PathBuf, SystemTime)> = None;
        let mut newest_branch: Option<(PathBuf, SystemTime)> = None;
        for entry in fs::read_dir(&current).ok()? {
            let Ok(entry) = entry else { continue };
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let Some(stamp) = entry.metadata().ok().and_then(|item| item.modified().ok()) else {
                continue;
            };
            let slot = if kind.is_dir() {
                &mut newest_branch
            } else if kind.is_file() {
                &mut newest_leaf
            } else {
                continue;
            };
            if slot.as_ref().is_none_or(|(_, best)| stamp > *best) {
                *slot = Some((entry.path(), stamp));
            }
        }
        if let Some((file, _)) = newest_leaf {
            return Some((file, current));
        }
        let (branch, _) = newest_branch?;
        current = branch;
    }
    None
}

struct Fallback {
    entries: Vec<FallbackEntry>,
}

impl Fallback {
    fn new(roots: Vec<Root>) -> Self {
        Self {
            entries: roots
                .into_iter()
                .map(|(provider, root)| FallbackEntry::new(provider, root))
                .collect(),
        }
    }

    fn poll(&mut self, state: &Observations) {
        for entry in &mut self.entries {
            entry.poll(state);
        }
    }
}

pub struct ActivityTracker {
    state: Arc<Observations>,
    /// Held only to keep the subscription alive; dropping it unsubscribes.
    _watcher: Mutex<Option<RecommendedWatcher>>,
    /// Polls the roots the subscription could not cover — all of them when it failed outright.
    fallback: Option<Mutex<Fallback>>,
}

impl ActivityTracker {
    fn start() -> Self {
        let state = Arc::new(Observations::default());
        let roots = registered_roots();
        let (watcher, unwatched) = start_watching(roots, Arc::clone(&state));
        let fallback = (!unwatched.is_empty()).then(|| Mutex::new(Fallback::new(unwatched)));
        Self {
            state,
            _watcher: Mutex::new(watcher),
            fallback,
        }
    }

    /// No-op while the subscription is live, which is the whole point: an idle machine does no
    /// filesystem work at all between events.
    fn refresh(&self) {
        let Some(fallback) = &self.fallback else {
            return;
        };
        if let Ok(mut fallback) = fallback.lock() {
            fallback.poll(&self.state);
        }
    }

    fn observations(&self) -> Vec<(&'static str, SystemTime)> {
        self.state
            .lock()
            .map(|observations| observations.iter().map(|(id, at)| (*id, *at)).collect())
            .unwrap_or_default()
    }
}

fn tracker() -> &'static ActivityTracker {
    static TRACKER: OnceLock<ActivityTracker> = OnceLock::new();
    TRACKER.get_or_init(ActivityTracker::start)
}

/// Subscribes at launch so the first widget poll already has history behind it.
pub fn start() {
    let _ = tracker();
}

/// The provider the user is currently working with, or `None` when no activity has been observed
/// and the caller should fall back to foreground-app attribution.
pub fn active_provider() -> Option<&'static str> {
    let tracker = tracker();
    tracker.refresh();
    let configured: Vec<&str> = super::configured()
        .iter()
        .map(|adapter| adapter.descriptor().id)
        .collect();
    select(&tracker.observations(), &configured)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    struct TempTree(PathBuf);

    impl TempTree {
        fn new(label: &str) -> Self {
            let stamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("cc-quota-activity-{label}-{stamp}"));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_most_recent_writer_wins() {
        let observed = [("alpha", at(100)), ("beta", at(300)), ("gamma", at(200))];
        assert_eq!(select(&observed, &["alpha", "beta", "gamma"]), Some("beta"));
    }

    /// A provider the user signed out of has nothing to display, so its activity must not win.
    #[test]
    fn skips_a_provider_without_a_local_login() {
        let observed = [("alpha", at(100)), ("beta", at(300)), ("gamma", at(200))];
        assert_eq!(select(&observed, &["alpha", "gamma"]), Some("gamma"));
        assert_eq!(select(&observed, &["alpha"]), Some("alpha"));
    }

    /// Nothing observed yet (a fresh install) must not guess: `None` sends the caller back to
    /// `classify_focus`, preserving the previous behaviour instead of inventing an answer.
    #[test]
    fn reports_nothing_when_no_activity_was_observed() {
        assert_eq!(select(&[], &["alpha", "beta"]), None);
        // Equally when every observed provider is signed out.
        assert_eq!(select(&[("alpha", at(100))], &[]), None);
    }

    #[test]
    fn attribution_ignores_paths_outside_the_session_directory() {
        let roots = vec![("alpha", PathBuf::from("/home/.alpha/sessions"))];
        assert_eq!(
            attribute(&roots, Path::new("/home/.alpha/sessions/2026/log.jsonl")),
            Some("alpha")
        );
        // The watcher may subscribe to the parent when the session directory does not exist yet;
        // credentials written next to it must not read as activity.
        assert_eq!(attribute(&roots, Path::new("/home/.alpha/auth.json")), None);
    }

    /// The fallback path, exercised as if the FSEvents subscription had failed.
    #[test]
    fn the_fallback_finds_activity_without_walking_the_whole_tree() {
        let tree = TempTree::new("fallback");
        let day = tree.0.join("2026").join("07").join("19");
        fs::create_dir_all(&day).unwrap();
        fs::write(day.join("session.jsonl"), b"x").unwrap();

        let state = Observations::default();
        let mut fallback = Fallback::new(vec![("alpha", tree.0.clone())]);
        fallback.poll(&state);

        let observed = state.lock().unwrap();
        assert!(
            observed.contains_key("alpha"),
            "fallback observed no activity"
        );
    }

    /// A prompt-history root is a single file: the fallback must stat it directly rather than try
    /// to descend into it as a tree.
    #[test]
    fn the_fallback_handles_a_file_root() {
        let tree = TempTree::new("file-root");
        let history = tree.0.join("history.jsonl");
        fs::write(&history, b"x").unwrap();

        let state = Observations::default();
        let mut fallback = Fallback::new(vec![("alpha", history)]);
        fallback.poll(&state);
        assert!(
            state.lock().unwrap().contains_key("alpha"),
            "fallback observed no activity for a file root"
        );
    }

    /// With a file root the watcher subscribes to the parent directory; a write to the file must
    /// still attribute to its provider. (`attribution_ignores_paths_outside_the_session_directory`
    /// covers the converse: the parent's other contents do not attribute.)
    #[test]
    fn the_watcher_attributes_a_file_root_through_its_parent() {
        let tree = TempTree::new("file-watch");
        let history = tree.0.join("history.jsonl");
        let state = Arc::new(Observations::default());
        let (watcher, unwatched) =
            start_watching(vec![("alpha", history.clone())], Arc::clone(&state));
        assert!(watcher.is_some(), "watch could not be established");
        assert!(unwatched.is_empty());

        fs::write(&history, b"x").unwrap();

        let deadline = SystemTime::now() + Duration::from_secs(10);
        while SystemTime::now() < deadline {
            if state.lock().unwrap().contains_key("alpha") {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("watcher observed no activity for a file root");
    }

    /// One root without an existing parent chain must not silence the fallback for itself: the
    /// watcher covers what it can and hands the rest back for polling. (Regression: the fallback
    /// used to be all-or-nothing, so a root created after launch went dark until restart.)
    #[test]
    fn an_unwatchable_root_is_returned_for_polling_not_dropped() {
        let tree = TempTree::new("mixed");
        let orphan = tree.0.join("missing-parent").join("history.jsonl");
        let state = Arc::new(Observations::default());
        let (watcher, unwatched) = start_watching(
            vec![("alpha", tree.0.clone()), ("beta", orphan)],
            Arc::clone(&state),
        );
        assert!(
            watcher.is_some(),
            "the healthy root should still be watched"
        );
        assert_eq!(
            unwatched
                .iter()
                .map(|(provider, _)| *provider)
                .collect::<Vec<_>>(),
            vec!["beta"]
        );
    }

    /// A provider whose CLI was never installed contributes nothing and must not error.
    #[test]
    fn the_fallback_tolerates_a_missing_directory() {
        let tree = TempTree::new("missing");
        let state = Observations::default();
        let mut fallback = Fallback::new(vec![("alpha", tree.0.join("never-created"))]);
        fallback.poll(&state);
        assert!(state.lock().unwrap().is_empty());
    }

    /// Only after a directory changes does the fallback walk again; otherwise it re-stats the file
    /// it already knows about.
    #[test]
    fn the_fallback_descends_to_the_newest_branch() {
        let tree = TempTree::new("descent");
        let old = tree.0.join("2025");
        let new = tree.0.join("2026");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("old.jsonl"), b"x").unwrap();
        fs::create_dir_all(&new).unwrap();
        fs::write(new.join("new.jsonl"), b"x").unwrap();

        let (file, directory) = newest_file(&tree.0).unwrap();
        assert_eq!(directory, new);
        assert_eq!(file, new.join("new.jsonl"));
    }

    /// Proves the primary path really is event-driven: nothing polls this directory, yet writing a
    /// file into a freshly created subdirectory shows up as activity.
    ///
    /// Doubles as the regression test for path normalisation — macOS puts the temp directory
    /// behind the `/var` → `/private/var` symlink, so without `resolve` every reported path fails
    /// to match its root and this observes nothing at all.
    #[test]
    fn the_watcher_records_a_write_without_anything_polling() {
        let tree = TempTree::new("watch");
        let state = Arc::new(Observations::default());
        let roots = vec![("alpha", tree.0.clone())];
        let (watcher, unwatched) = start_watching(roots, Arc::clone(&state));
        assert!(watcher.is_some(), "watch could not be established");
        assert!(unwatched.is_empty());

        let day = tree.0.join("2026").join("07").join("19");
        fs::create_dir_all(&day).unwrap();
        fs::write(day.join("session.jsonl"), b"x").unwrap();

        // FSEvents coalesces, so the event arrives shortly rather than instantly.
        let deadline = SystemTime::now() + Duration::from_secs(10);
        while SystemTime::now() < deadline {
            if state.lock().unwrap().contains_key("alpha") {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("watcher observed no activity");
    }

    /// Every registered provider must name at least one directory, or it can never be detected as
    /// active. Paths that do not exist locally are fine — the user may not have that CLI.
    #[test]
    fn every_provider_declares_an_activity_directory() {
        for adapter in super::super::all() {
            assert!(
                !adapter.activity_paths().is_empty(),
                "{} declares no activity directory",
                adapter.descriptor().id
            );
        }
    }
}
