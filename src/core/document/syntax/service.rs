//! Bounded fair scheduling. UI/coordinator callers only copy immutable handles,
//! replace a mailbox request, and install completed packages. All provider
//! loading and analysis happens outside mailbox locks on worker threads.
use super::{Coverage, SyntaxInputIdentity, SyntaxInputSnapshot, SyntaxRun};
use std::{
    collections::VecDeque,
    ops::Range,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex, OnceLock, Weak,
    },
    time::{Duration, Instant},
};

mod providers;
mod retention;
mod runs;
/// Fuzz targets drive the worker-side provider synchronously.
#[cfg(any(fuzzing, test))]
pub use providers::BackendProvider;
pub use runs::{PublicationDelta, RunStore};
pub const MAX_REGION_BYTES: usize = 256 * 1024;
pub const MAX_CACHED_REGIONS: usize = 16;
pub const MAX_CACHED_RUN_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_QUEUE_BUFFERS: usize = 128;
pub const WORKER_COUNT: usize = 2;
pub const MAX_IDLE_PROVIDER_SESSIONS: usize = 8;
/// Shared retained analysis, including frozen text inputs, across idle sessions.
/// Native active work remains subject to the separate cooperative native cap.
pub const MAX_IDLE_PROVIDER_BYTES: usize = 512 * 1024 * 1024;
pub const MAX_CONTINUATION_SLICES: usize = 4096;
/// Maximum grace period before presenting newly visible text without syntax.
pub const SYNTAX_PRESENTATION_WAIT: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntaxConfiguration {
    pub generation: u64,
    pub registry_generation: u64,
    pub language: Option<String>,
    pub vim_directory: String,
    pub filename: Option<String>,
}
impl Default for SyntaxConfiguration {
    fn default() -> Self {
        Self {
            generation: 1,
            registry_generation: super::treesitter::package_registry_generation(),
            language: None,
            filename: None,
            // Frontends resolve bundled resources or a user-selected directory.
            vim_directory: String::new(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct SyntaxRequest {
    pub input: SyntaxInputSnapshot,
    pub configuration: SyntaxConfiguration,
    pub range: Range<usize>,
}
impl SyntaxRequest {
    fn same(&self, other: &Self) -> bool {
        self.input.identity() == other.input.identity()
            && self.configuration == other.configuration
            && self.range == other.range
    }
}
#[derive(Clone, Debug)]
pub struct SyntaxResult {
    pub input: SyntaxInputIdentity,
    pub configuration: SyntaxConfiguration,
    pub range: Range<usize>,
    pub runs: Vec<SyntaxRun>,
    pub coverage: Coverage,
    pub diagnostics: Vec<String>,
    /// True only when a private frozen continuation can make more progress.
    pub continuation: bool,
}
impl SyntaxResult {
    pub fn missing(request: &SyntaxRequest, message: impl Into<String>) -> Self {
        Self {
            input: request.input.identity(),
            configuration: request.configuration.clone(),
            range: request.range.clone(),
            runs: Vec::new(),
            coverage: Coverage::Missing,
            diagnostics: vec![message.into()],
            continuation: false,
        }
    }
    fn bytes(&self) -> usize {
        self.runs
            .iter()
            .map(|r| std::mem::size_of::<SyntaxRun>() + r.name.0.len() + r.origin.len())
            .sum::<usize>()
            + self.diagnostics.iter().map(String::len).sum::<usize>()
    }
}

/// Platform integrations use the same immutable coverage contract. Mutable
/// sessions are exclusively worker-owned and may not call back into a core.
pub trait SyntaxProvider: Send {
    /// Conservative retained allocation charge for worker-owned state. Called
    /// outside mailbox locks; stateless providers have no retained allocations.
    fn retained_bytes(&self) -> usize { 0 }

    fn analyze(&mut self, request: &SyntaxRequest, cancellation: &AtomicBool) -> SyntaxResult;
}
/// A factory executes on a syntax worker. Platform adapters can return a
/// bounded native session or a proxy to an isolated process executor.
pub type SyntaxProviderFactory = Arc<dyn Fn() -> Box<dyn SyntaxProvider> + Send + Sync>;
type Factory = SyntaxProviderFactory;
struct Slot {
    progress: Arc<Condvar>,
    factory: Factory,
    pending: Option<SyntaxRequest>,
    active: Option<SyntaxRequest>,
    ready: Option<SyntaxResult>,
    cancellation: Arc<AtomicBool>,
    queued: bool,
    running: bool,
    closed: Arc<AtomicBool>,
    wake: crate::background::WorkWake,
    continuation_key: Option<(SyntaxInputIdentity, SyntaxConfiguration)>,
    continuation_slices: usize,
}
type Mailbox = Arc<Mutex<Slot>>;

/// A snapshot-specific wait handle. It owns no core/view state and must only be
/// used after releasing the coordinator lease. Condvar waiting releases the
/// mailbox lock, so provider completion and cancellation remain independent.
pub struct SyntaxWait {
    mailbox: Mailbox,
    request: SyntaxRequest,
    cancellation: Arc<AtomicBool>,
    deadline: Instant,
}

/// View-local scheduling state, containing no text or cached highlighting.
/// A repaint of the same viewport cannot restart an expired grace period, and
/// another view cannot overwrite its deadline.
#[derive(Default)]
pub struct SyntaxWaitState {
    target: Option<(SyntaxInputIdentity, SyntaxConfiguration, Range<usize>, Instant)>,
}
impl SyntaxWaitState {
    /// Editing text already presented in this view does not expose a cold
    /// viewport. The worker still receives the new revision; only the wait
    /// is retired while the rebased, provisional colors are on screen.
    pub(crate) fn rebase_visible_edit(&mut self, map: &crate::document::PositionMap,
        change: &crate::document::LocalFormattedChange) {
        let Some((input, _, range, deadline)) = &mut self.target else { return };
        if input.document != map.document().0 || input.revision != map.source_revision().0
            || change.old.start != change.new.start || change.old.start < range.start || change.old.end > range.end {
            return;
        }
        range.end = range.end - change.old.len() + change.new.len();
        input.revision = map.target_revision().0;
        *deadline = Instant::now();
    }

    fn deadline(&mut self, request: &SyntaxRequest, deadline: Instant) -> Instant {
        if let Some((input, configuration, range, deadline)) = &mut self.target {
            if *input == request.input.identity() && *configuration == request.configuration
                && range.start <= request.range.start && request.range.end <= range.end {
                *range = request.range.clone();
                return *deadline;
            }
        }
        let deadline = deadline.min(Instant::now() + SYNTAX_PRESENTATION_WAIT);
        self.target = Some((request.input.identity(), request.configuration.clone(), request.range.clone(), deadline));
        deadline
    }
}
impl SyntaxWait {
    pub fn same_target(&self, other: &Self) -> bool {
        self.request.same(&other.request)
    }

    /// Wait for one publication, cancellation, or the caller's total deadline.
    /// Continuation slices never reset that deadline.
    pub fn wait_until(&self, deadline: Instant) -> bool {
        let deadline = deadline.min(self.deadline);
        let mut slot = self.mailbox.lock().unwrap_or_else(|e| e.into_inner());
        let progress = slot.progress.clone();
        loop {
            if slot.closed.load(Ordering::Acquire) || self.cancellation.load(Ordering::Acquire) { return false; }
            if slot.ready.is_some() { return true; }
            let pending = slot.pending.as_ref().is_some_and(|r| r.same(&self.request));
            let active = slot.active.as_ref().is_some_and(|r| r.same(&self.request));
            if !pending && !active { return false; }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            slot = progress.wait_timeout(slot, remaining)
                .unwrap_or_else(|e| e.into_inner()).0;
        }
    }
}
struct IdleProvider {
    mailbox: Weak<Mutex<Slot>>,
    closed: Arc<AtomicBool>,
    provider: Box<dyn SyntaxProvider>,
    bytes: usize,
}
struct ScheduledMailbox {
    mailbox: Weak<Mutex<Slot>>,
    closed: Arc<AtomicBool>,
}
#[derive(Default)]
struct Schedule {
    queue: VecDeque<ScheduledMailbox>,
    /// Overflow contains only a weak mailbox and its close flag, never another
    /// request or snapshot. A mailbox reserves its unique entry under its own
    /// lock before registering here. Thus total scheduling entries are bounded
    /// by live services, with at most MAX_QUEUE_BUFFERS admitted at once.
    waiting: VecDeque<ScheduledMailbox>,
    /// Durable predicate: close must not be lost between cleanup and parking,
    /// even when a detached SyntaxWait keeps the mailbox alive.
    retirement_requested: bool,
}
impl Schedule {
    fn admit_waiters(&mut self) {
        while self.queue.len() < MAX_QUEUE_BUFFERS {
            let Some(entry) = self.waiting.pop_front() else { break; };
            if !entry.closed.load(Ordering::Acquire) {
                self.queue.push_back(entry);
            }
        }
        // A temporary burst of documents/code blocks must not retain its peak
        // scheduling allocation forever after the services settle or close.
        let capacity = self.waiting.capacity();
        if capacity > MAX_QUEUE_BUFFERS && self.waiting.len() <= capacity / 4 {
            self.waiting.shrink_to(self.waiting.len().max(MAX_QUEUE_BUFFERS));
        }
    }
}
#[derive(Default)]
struct Pool {
    schedule: Mutex<Schedule>,
    available: Condvar,
    idle: Mutex<VecDeque<IdleProvider>>,
}
static POOL: OnceLock<Arc<Pool>> = OnceLock::new();
fn pool() -> &'static Arc<Pool> {
    POOL.get_or_init(|| {
        let pool = Arc::new(Pool::default());
        for index in 0..WORKER_COUNT {
            let worker = pool.clone();
            std::thread::Builder::new()
                .name(format!("viem-syntax-{index}"))
                .spawn(move || worker.run())
                .expect("syntax worker creation");
        }
        pool
    })
}
impl Pool {
    fn take_provider(&self, mailbox: &Mailbox) -> Option<Box<dyn SyntaxProvider>> {
        let mut idle = self.idle.lock().unwrap_or_else(|e| e.into_inner());
        let key = Arc::downgrade(mailbox);
        let index = idle.iter().position(|entry| entry.mailbox.ptr_eq(&key))?;
        Some(idle.remove(index).unwrap().provider)
    }
    fn retain_provider(
        &self,
        mailbox: &Mailbox,
        provider: Box<dyn SyntaxProvider>,
        bytes: usize,
    ) -> Vec<IdleProvider> {
        let closed = mailbox.lock().unwrap_or_else(|e| e.into_inner()).closed.clone();
        let entry = IdleProvider { mailbox: Arc::downgrade(mailbox), closed, provider, bytes };
        let mut idle = self.idle.lock().unwrap_or_else(|e| e.into_inner());
        // Check under the cache lock: a close either prevents insertion here,
        // or its durable retirement request observes the inserted session.
        if entry.closed.load(Ordering::Acquire) { return vec![entry]; }
        idle.push_back(entry);
        let mut retained = idle.iter().map(|entry| entry.bytes).sum::<usize>();
        let mut retired = Vec::new();
        while idle.len() > MAX_IDLE_PROVIDER_SESSIONS || retained > MAX_IDLE_PROVIDER_BYTES {
            let Some(entry) = idle.pop_front() else { break; };
            retained = retained.saturating_sub(entry.bytes);
            retired.push(entry);
        }
        retired
    }
    fn retire_closed(&self) {
        let retired = {
            let mut idle = self.idle.lock().unwrap_or_else(|e| e.into_inner());
            let mut retired = Vec::new();
            for index in (0..idle.len()).rev() {
                if idle[index].closed.load(Ordering::Acquire) {
                    retired.push(idle.remove(index).unwrap());
                }
            }
            retired
        };
        drop(retired);
    }
    /// The caller first reserves Slot::queued, then releases the mailbox lock.
    /// Every pending mailbox joins the fair queue exactly once, including when
    /// admission is full. Workers promote overflow as capacity becomes free.
    fn enqueue(&self, mailbox: &Mailbox, closed: Arc<AtomicBool>) {
        let mut schedule = self.schedule.lock().unwrap_or_else(|e| e.into_inner());
        if closed.load(Ordering::Acquire) { return; }
        let entry = ScheduledMailbox { mailbox: Arc::downgrade(mailbox), closed };
        if schedule.waiting.is_empty() && schedule.queue.len() < MAX_QUEUE_BUFFERS {
            schedule.queue.push_back(entry);
        } else {
            schedule.waiting.push_back(entry);
        }
        drop(schedule);
        self.available.notify_one();
    }
    fn close(&self, mailbox: &Mailbox) {
        let key = Arc::downgrade(mailbox);
        let mut schedule = self.schedule.lock().unwrap_or_else(|e| e.into_inner());
        schedule.queue.retain(|entry| !entry.mailbox.ptr_eq(&key));
        schedule.waiting.retain(|entry| !entry.mailbox.ptr_eq(&key));
        schedule.admit_waiters();
        schedule.retirement_requested = true;
        drop(schedule);
        self.available.notify_one();
    }
    fn take_work(&self) -> (Option<Mailbox>, bool) {
        let mut schedule = self.schedule.lock().unwrap_or_else(|e| e.into_inner());
        while schedule.queue.is_empty() && !schedule.retirement_requested {
            schedule = self.available.wait(schedule).unwrap_or_else(|e| e.into_inner());
        }
        let retire = std::mem::take(&mut schedule.retirement_requested);
        let mailbox = schedule.queue.pop_front().and_then(|entry| entry.mailbox.upgrade());
        schedule.admit_waiters();
        (mailbox, retire)
    }
    fn run(&self) {
        loop {
            let (mailbox, retire) = self.take_work();
            if retire { self.retire_closed(); }
            let Some(mailbox) = mailbox else { continue; };
            let job = {
                let mut slot = mailbox.lock().unwrap_or_else(|e| e.into_inner());
                slot.queued = false;
                if slot.closed.load(Ordering::Acquire) {
                    continue;
                }
                let Some(request) = slot.pending.take() else {
                    continue;
                };
                slot.running = true;
                slot.active = Some(request.clone());
                let key = (request.input.identity(), request.configuration.clone());
                if slot.continuation_key.as_ref() != Some(&key) {
                    slot.continuation_key = Some(key);
                    slot.continuation_slices = 0;
                }
                let capped = slot.continuation_slices >= MAX_CONTINUATION_SLICES;
                Some((
                    request,
                    slot.factory.clone(),
                    slot.cancellation.clone(),
                    capped,
                ))
            };
            let Some((request, factory, cancellation, capped)) = job else {
                continue;
            };
            let mut provider = self.take_provider(&mailbox);
            let computed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if capped {
                    SyntaxResult::missing(&request, "Shared syntax continuation budget exhausted")
                } else {
                    provider
                        .get_or_insert_with(|| factory())
                        .analyze(&request, &cancellation)
                }
            }));
            if computed.is_err() {
                provider = None;
            }
            let mut result = computed
                .unwrap_or_else(|_| SyntaxResult::missing(&request, "Syntax provider failed"));
            if result.bytes() > MAX_CACHED_RUN_BYTES || result.runs.len() > 32_768 {
                result = SyntaxResult::missing(&request, "Syntax result exceeded output budget");
            }
            let mut end = result.range.start;
            let valid = result.input == request.input.identity()
                && result.configuration == request.configuration
                && result.configuration.registry_generation
                    == super::treesitter::package_registry_generation()
                && result.range.start >= request.range.start
                && result.range.end <= request.range.end
                && result.range.start <= result.range.end
                && result.runs.iter().all(|run| {
                    let valid = run.range.start >= end
                        && run.range.start < run.range.end
                        && run.range.end <= result.range.end
                        && request
                            .input
                            .text_tree()
                            .is_char_boundary(run.range.start)
                            .unwrap_or(false)
                        && request
                            .input
                            .text_tree()
                            .is_char_boundary(run.range.end)
                            .unwrap_or(false);
                    end = run.range.end;
                    valid
                });
            if !valid {
                result = SyntaxResult::missing(
                    &request,
                    "Syntax provider returned invalid snapshot coverage",
                );
            }
            let provider_bytes = provider.as_ref().map_or(0, |provider| provider.retained_bytes());
            // A continuation too large to retain cannot make progress by
            // repeatedly rebuilding a fresh session on every worker slice.
            if provider_bytes > MAX_IDLE_PROVIDER_BYTES {
                provider = None;
                result.continuation = false;
                result.diagnostics.push("Syntax session exceeded retained memory budget".into());
            }
            // Keep running=true until the provider is retained, so another
            // request cannot admit this mailbox while its session is in transit.
            // Mailbox, pool and cache locks are never nested.
            let retired = provider.map(|provider| self.retain_provider(&mailbox, provider, provider_bytes)).unwrap_or_default();
            let mut slot = mailbox.lock().unwrap_or_else(|e| e.into_inner());
            slot.running = false;
            slot.active = None;
            if slot.closed.load(Ordering::Acquire) {
                drop(slot);
                drop(retired);
                continue;
            }
            let wake = if !cancellation.load(Ordering::Acquire) {
                if result.continuation {
                    slot.continuation_slices += 1;
                    if slot.continuation_slices >= MAX_CONTINUATION_SLICES {
                        result.continuation = false;
                        result.diagnostics.push("Shared syntax continuation budget exhausted".into());
                    }
                }
                if result.continuation && slot.pending.is_none() {
                    slot.pending = Some(request);
                }
                slot.ready = Some(result);
                Some(slot.wake.clone())
            } else { None };
            let enqueue = slot.pending.is_some() && !slot.queued;
            if enqueue { slot.queued = true; }
            let closed = slot.closed.clone();
            slot.progress.notify_all();
            drop(slot);
            if enqueue { self.enqueue(&mailbox, closed); }
            if let Some(wake) = wake { wake.signal(); }
            // Native destruction is worker-only and outside every lock.
            drop(retired);
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SyntaxServiceStatistics {
    pub requests: u64,
    pub cache_hits: u64,
    pub stale_rejections: u64,
    pub publications: u64,
    pub maximum_pending: usize,
}
struct VisibleDemand {
    range: Range<usize>,
    satisfied: bool,
}
struct VisibleDemands {
    input: SyntaxInputIdentity,
    configuration: SyntaxConfiguration,
    /// Sorted and distinct; only the owner's current ranges are retained.
    ranges: Vec<VisibleDemand>,
}

pub struct SyntaxService {
    mailbox: Mailbox,
    pub configuration: SyntaxConfiguration,
    cache: VecDeque<SyntaxResult>,
    visible_demands: Option<VisibleDemands>,
    current: Option<SyntaxInputIdentity>,
    current_input: Option<SyntaxInputSnapshot>,
    /// The current input's displayed runs: accepted results plus appearance
    /// mapped from earlier inputs. Presentation only; runs cannot satisfy
    /// analysis requests, which accepted coverage and current demands decide.
    runs: RunStore,
    /// What changed in `runs` since the last publication took it.
    delta: PublicationDelta,
    pub statistics: SyntaxServiceStatistics,
    diagnostics: Vec<String>,
    registry_generation: u64,
}
impl Default for SyntaxService {
    fn default() -> Self {
        Self::with_factory(Arc::new(|| Box::new(providers::BackendProvider::default())))
    }
}
impl SyntaxService {
    /// Capture the same provider/configuration for an independent export. The
    /// clone has no viewport cache, and all provider work still runs on the
    /// shared syntax workers. It does not disturb the editor's pending request.
    pub(crate) fn for_export(&self) -> Self {
        let (factory, wake) = {
            let slot = self.mailbox.lock().unwrap_or_else(|e| e.into_inner());
            (slot.factory.clone(), slot.wake.clone())
        };
        let mut service = Self::with_factory(factory);
        service.set_work_wake(wake);
        service.configuration = self.configuration.clone();
        service
    }

    /// Whole-document consumers explicitly visit every bounded region instead
    /// of using the viewport's incomplete, evictable presentation cache. Call
    /// only after releasing the coordinator lease; this waits for workers.
    pub(crate) fn export_runs(mut self, input: SyntaxInputSnapshot) -> Vec<SyntaxRun> {
        // Prepared view services inherit the owner's notifier; a true export
        // waits independently and must not wake that owner's presentation loop.
        self.set_work_wake(crate::background::WorkWake::default());
        let mut output = Vec::new();
        let mut start = 0;
        while start < input.byte_len() {
            let mut end = (start + MAX_REGION_BYTES).min(input.byte_len());
            while !input.text_tree().is_char_boundary(end).unwrap_or(false) { end -= 1; }
            let mut pending = VecDeque::from([start..end]);
            let mut slices = 0;
            while let Some(range) = pending.pop_front() {
                // A provider that reports tiny final subsets cannot turn one
                // export region into unbounded work. Unavailable coverage keeps
                // the normal Code appearance, just as it does in the editor.
                slices += 1;
                if slices > MAX_CONTINUATION_SLICES { break; }
                loop {
                    self.poll(input.identity());
                    let completed = self.cache.iter().rev().find(|result|
                        result.input == input.identity()
                            && result.configuration == self.configuration
                            && result.range.start >= range.start && result.range.end <= range.end
                            && (result.range.is_empty() || result.range.start < range.end)
                            && !result.continuation)
                        .map(|result| (result.range.clone(), result.coverage));
                    if let Some((covered, coverage)) = completed {
                        let accepted = if coverage == Coverage::Missing { range.clone() } else { covered.clone() };
                        for mut run in self.run_store().runs_in(&accepted) {
                            run.range = run.range.start.max(accepted.start)..run.range.end.min(accepted.end);
                            if !run.range.is_empty() { output.push(run); }
                        }
                        // Native providers may publish a final strict subset.
                        // Visit both uncovered sides rather than silently using
                        // their absent viewport styles for the rest of the file.
                        if coverage != Coverage::Missing && !covered.is_empty() {
                            if range.start < covered.start { pending.push_back(range.start..covered.start); }
                            if covered.end < range.end { pending.push_back(covered.end..range.end); }
                        }
                        break;
                    }
                    self.request(input.clone(), range.clone());
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            }
            start = end;
        }
        output.sort_by_key(|run| run.range.start);
        output
    }

    pub fn with_factory(factory: Factory) -> Self {
        Self {
            mailbox: Arc::new(Mutex::new(Slot {
                progress: Arc::new(Condvar::new()),
                factory,
                pending: None,
                active: None,
                ready: None,
                cancellation: Arc::new(AtomicBool::new(false)),
                queued: false,
                running: false,
                closed: Arc::new(AtomicBool::new(false)),
                wake: crate::background::WorkWake::default(),
                continuation_key: None,
                continuation_slices: 0,
            })),
            configuration: Default::default(),
            cache: VecDeque::new(),
            visible_demands: None,
            current: None,
            current_input: None,
            runs: RunStore::default(),
            delta: PublicationDelta::unbounded(),
            statistics: Default::default(),
            diagnostics: Vec::new(),
            registry_generation: 0,
        }
    }
    /// Bind the owning document's coalesced completion notification. Workers
    /// invoke it only after releasing all mailbox, scheduler and cache locks.
    pub fn set_work_wake(&mut self, wake: crate::background::WorkWake) {
        let previous = {
            let mut slot = self.mailbox.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::replace(&mut slot.wake, wake)
        };
        drop(previous);
    }

    pub fn has_ready_work(&self) -> bool {
        self.mailbox.lock().unwrap_or_else(|e| e.into_inner()).ready.is_some()
    }

    pub fn set_language(&mut self, language: Option<String>) {
        if self.configuration.language != language {
            self.configuration.language = language;
            self.invalidate_configuration();
        }
    }
    pub fn set_vim_directory(&mut self, path: String) {
        if self.configuration.vim_directory != path {
            self.configuration.vim_directory = path;
            self.invalidate_configuration();
        }
    }
    pub fn set_filename(&mut self, filename: Option<String>) {
        if self.configuration.filename != filename {
            self.configuration.filename = filename;
            self.invalidate_configuration();
        }
    }
    fn invalidate_configuration(&mut self) {
        self.configuration.generation = self.configuration.generation.wrapping_add(1).max(1);
        self.cancel();
        self.cache.clear();
        self.visible_demands = None;
        self.current = None;
        self.current_input = None;
        self.runs = RunStore::default();
        self.delta = PublicationDelta::unbounded();
    }
    pub fn cancel(&mut self) {
        let mut slot = self.mailbox.lock().unwrap_or_else(|e| e.into_inner());
        slot.cancellation.store(true, Ordering::Release);
        slot.pending = None;
        slot.ready = None;
        slot.progress.notify_all();
    }
    fn request_range(input: &SyntaxInputSnapshot, mut range: Range<usize>) -> Option<Range<usize>> {
        if range.start > range.end || range.end > input.byte_len()
            || !input.text_tree().is_char_boundary(range.start).unwrap_or(false) {
            return None;
        }
        range.end = range.end.min(range.start.saturating_add(MAX_REGION_BYTES));
        while range.end > range.start
            && !input.text_tree().is_char_boundary(range.end).unwrap_or(false) {
            range.end -= 1;
        }
        Some(range)
    }
    /// Replace the owner's current visible demands. Completed exact requests
    /// remain settled even after their coverage metadata leaves the small cache;
    /// otherwise more simultaneous views than cache entries would continuously
    /// request and evict each other's results. Runs retain their separate budget.
    /// Only current distinct ranges are stored, with no extra text or run copies.
    pub(crate) fn set_visible_demands(&mut self, input: &SyntaxInputSnapshot, ranges: &[Range<usize>]) {
        // Even a no-view buffer with complete analysis must notice replacement
        // packages before deciding that no new requests are necessary.
        self.refresh_registry_configuration();
        let previous = self.visible_demands.take().filter(|previous|
            previous.input == input.identity() && previous.configuration == self.configuration);
        let mut ranges = ranges.iter().filter_map(|range| Self::request_range(input, range.clone())).collect::<Vec<_>>();
        ranges.sort_unstable_by_key(|range| (range.start, range.end));
        ranges.dedup();
        let ranges = ranges.into_iter().map(|range| {
            let satisfied = previous.as_ref().and_then(|previous| {
                previous.ranges.binary_search_by_key(&(range.start, range.end), |demand| (demand.range.start, demand.range.end))
                    .ok().map(|index| previous.ranges[index].satisfied)
            }).unwrap_or(false) || self.has_cached_coverage(input.identity(), &range);
            VisibleDemand { range, satisfied }
        }).collect();
        self.visible_demands = Some(VisibleDemands { input: input.identity(), configuration: self.configuration.clone(), ranges });
    }

    fn settle_visible_demands(&mut self) {
        let Some(mut demands) = self.visible_demands.take() else { return; };
        if demands.configuration == self.configuration {
            for demand in &mut demands.ranges {
                demand.satisfied |= self.has_cached_coverage(demands.input, &demand.range);
            }
        }
        self.visible_demands = Some(demands);
    }

    /// Whether completed analysis of `input` covers `range`, as a request
    /// would normalize it.
    pub(crate) fn covers(&self, input: &SyntaxInputSnapshot, range: Range<usize>) -> bool {
        Self::request_range(input, range).is_some_and(|range| self.has_coverage(input.identity(), &range))
    }
    /// A terminal failure settles only its own request. A smaller region may
    /// still fit the budget that the failed one exceeded, so it never stands in
    /// for coverage of its subranges.
    fn has_coverage(&self, input: SyntaxInputIdentity, range: &Range<usize>) -> bool {
        let satisfied = self.visible_demands.as_ref().filter(|demands|
            demands.input == input && demands.configuration == self.configuration)
            .and_then(|demands| demands.ranges.binary_search_by_key(&(range.start, range.end),
                |demand| (demand.range.start, demand.range.end)).ok().map(|index| demands.ranges[index].satisfied))
            .unwrap_or(false);
        satisfied || self.has_cached_coverage(input, range)
    }
    fn has_cached_coverage(&self, input: SyntaxInputIdentity, range: &Range<usize>) -> bool {
        let mut covered = range.start;
        loop {
            let before = covered;
            // The bounded cache is in eviction order. Adjacent results can
            // cover the viewport together even if neither covers it alone.
            for result in &self.cache {
                if result.input != input || result.configuration != self.configuration { continue; }
                if result.coverage == Coverage::Missing {
                    if result.range == *range { return true; }
                } else if result.range.start <= covered {
                    if result.range.end >= range.end { return true; }
                    covered = covered.max(result.range.end);
                }
            }
            if covered == before { return false; }
        }
    }
    /// Request only missing visible coverage and return a detached wait handle.
    /// Exact empty captures and terminal failure both count as ready: neither
    /// should impose another grace period on subsequent frames.
    pub fn prepare_wait(&mut self, input: SyntaxInputSnapshot, range: Range<usize>, state: &mut SyntaxWaitState, deadline: Instant) -> Option<SyntaxWait> {
        let range = Self::request_range(&input, range)?;
        if range.is_empty() || self.configuration.language.is_none() { return None; }
        self.request(input.clone(), range.clone());
        let request = SyntaxRequest {
            input, configuration: self.configuration.clone(), range,
        };
        let deadline = state.deadline(&request, deadline);
        if Instant::now() >= deadline || self.has_coverage(request.input.identity(), &request.range) { return None; }
        let cancellation = self.mailbox.lock().unwrap_or_else(|e| e.into_inner()).cancellation.clone();
        Some(SyntaxWait { mailbox: self.mailbox.clone(), request, cancellation, deadline })
    }
    fn refresh_registry_configuration(&mut self) {
        let registry = super::treesitter::package_registry_generation();
        if registry != self.registry_generation {
            self.registry_generation = registry;
            self.configuration.registry_generation = registry;
            self.invalidate_configuration();
        }
    }
    pub fn request(&mut self, input: SyntaxInputSnapshot, range: Range<usize>) {
        self.refresh_registry_configuration();
        let Some(range) = Self::request_range(&input, range) else { return; };
        if self.current != Some(input.identity()) {
            self.rebase_input(input.clone(), None, None);
        }
        if self.has_coverage(input.identity(), &range) {
            self.statistics.cache_hits += 1;
            return;
        }
        let request = SyntaxRequest {
            input,
            configuration: self.configuration.clone(),
            range,
        };
        let mut slot = self.mailbox.lock().unwrap_or_else(|e| e.into_inner());
        // Completion may race the coordinator's preceding poll. Leave that
        // ready result for publication instead of rerunning identical coverage.
        if slot.ready.as_ref().is_some_and(|ready| {
            ready.input == request.input.identity()
                && ready.configuration == request.configuration
                && ready.range.start <= request.range.start
                && ready.range.end >= request.range.end
        }) { return; }
        if slot.pending.as_ref().is_some_and(|r| r.same(&request)) { return; }
        if slot.active.as_ref().is_some_and(|r| r.same(&request)) {
            return;
        }
        if slot.active.as_ref().is_some_and(|r| {
            r.input.identity() != request.input.identity()
                || r.configuration != request.configuration
        }) {
            slot.cancellation.store(true, Ordering::Release);
            slot.cancellation = Arc::new(AtomicBool::new(false));
        } else if slot.cancellation.load(Ordering::Acquire) {
            slot.cancellation = Arc::new(AtomicBool::new(false));
        }
        slot.pending = Some(request);
        self.statistics.requests += 1;
        self.statistics.maximum_pending = 1;
        let enqueue = !slot.running && !slot.queued;
        if enqueue { slot.queued = true; }
        let closed = slot.closed.clone();
        slot.progress.notify_all();
        drop(slot);
        if enqueue { pool().enqueue(&self.mailbox, closed); }
    }
    pub fn poll(&mut self, input: SyntaxInputIdentity) -> bool {
        let result = self
            .mailbox
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .ready
            .take();
        let Some(result) = result else {
            return false;
        };
        if result.input != input
            || result.configuration != self.configuration
            || result.configuration.registry_generation
                != super::treesitter::package_registry_generation()
        {
            self.statistics.stale_rejections += 1;
            return false;
        }
        self.diagnostics = result.diagnostics.clone();
        if result.continuation && result.coverage == Coverage::Missing {
            return false;
        }
        // A result for coverage the cache does not hold yet is a publication
        // even when the displayed runs are unchanged, so coverage state and
        // diagnostics reach the coordinator.
        let novel = !self.cache.iter().any(|old| {
            old.input == result.input
                && old.configuration == result.configuration
                && old.range == result.range
                && old.coverage == result.coverage
        });
        let appearance_changed = self.replace_presentation_coverage(&result) || novel;
        // The store now holds the runs; the cache entry keeps only coverage.
        let mut coverage = result;
        coverage.runs = Vec::new();
        self.cache.push_back(coverage);
        // Record completion before bounding metadata: cache eviction must not
        // turn an unchanged, already-served viewport into another request.
        self.settle_visible_demands();
        self.bound_presentation_cache();
        if appearance_changed {
            self.statistics.publications += 1;
        }
        appearance_changed
    }
    /// Every displayed run of `input` in start order. This flattens the run
    /// store; publications use `run_store` and `take_publication_delta`.
    pub fn runs(&self, input: SyntaxInputIdentity) -> Vec<SyntaxRun> {
        if self.current == Some(input) {
            self.runs.runs()
        } else {
            Vec::new()
        }
    }
    /// The indexed displayed runs of the current input.
    pub fn run_store(&self) -> &RunStore {
        &self.runs
    }
    /// The regions whose runs changed since the previous call, resetting the
    /// record. The first call after a new input or configuration is unbounded.
    pub fn take_publication_delta(&mut self) -> PublicationDelta {
        std::mem::take(&mut self.delta)
    }
    /// Distinct names referenced by the displayed runs, sorted.
    pub fn referenced_names(&self) -> Vec<String> {
        self.runs.names().map(|name| name.to_string()).collect()
    }
    pub fn diagnostics(&self) -> String {
        self.diagnostics.join("\n")
    }
    pub fn retained_result_bytes(&self) -> usize {
        self.cache.iter().map(SyntaxResult::bytes).sum::<usize>() + self.runs.bytes()
    }
    /// Include a completed result until its owner has polled and accepted it.
    /// This is a bounded scheduling query; no provider work occurs under the lock.
    pub(crate) fn has_pending_work(&self) -> bool {
        let slot = self.mailbox.lock().unwrap_or_else(|e| e.into_inner());
        slot.pending.is_some() || slot.active.is_some() || slot.ready.is_some()
    }
}
impl Drop for SyntaxService {
    fn drop(&mut self) {
        let mut slot = self.mailbox.lock().unwrap_or_else(|e| e.into_inner());
        slot.closed.store(true, Ordering::Release);
        slot.cancellation.store(true, Ordering::Release);
        slot.pending = None;
        slot.ready = None;
        slot.progress.notify_all();
        drop(slot);
        if let Some(pool) = POOL.get() { pool.close(&self.mailbox); }
    }
}

#[cfg(test)]
mod wait_tests;
#[cfg(test)]
mod scheduling_tests;
#[cfg(test)]
mod demand_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::FormattedTextTree;
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    struct Controlled {
        started: mpsc::Sender<u64>,
        gate: Arc<(Mutex<bool>, Condvar)>,
        first: bool,
    }
    impl SyntaxProvider for Controlled {
        fn analyze(&mut self, r: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
            assert!(std::thread::current()
                .name()
                .unwrap_or("")
                .starts_with("viem-syntax-"));
            self.started.send(r.input.identity().revision).unwrap();
            if self.first {
                self.first = false;
                let (lock, cv) = &*self.gate;
                let mut open = lock.lock().unwrap();
                while !*open {
                    open = cv.wait(open).unwrap();
                }
            }
            SyntaxResult {
                input: r.input.identity(),
                configuration: r.configuration.clone(),
                range: r.range.clone(),
                runs: Vec::new(),
                coverage: Coverage::Exact,
                diagnostics: Vec::new(),
                continuation: false,
            }
        }
    }
    pub(super) fn input(revision: u64) -> SyntaxInputSnapshot {
        SyntaxInputSnapshot::new(
            SyntaxInputIdentity {
                document: 7,
                revision,
                generation: 1,
            },
            FormattedTextTree::try_from_text("x\n".repeat(1000)).unwrap(),
        )
    }
    #[test]
    fn retained_provider_pool_evicts_by_bytes_and_reuses_surviving_sessions() {
        struct SizedProvider(usize);
        impl SyntaxProvider for SizedProvider {
            fn retained_bytes(&self) -> usize { self.0 }
            fn analyze(&mut self, request: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
                SyntaxResult::missing(request, "fixture")
            }
        }
        let local = Pool::default();
        let factory: Factory = Arc::new(|| Box::new(SizedProvider(0)));
        let first = SyntaxService::with_factory(factory.clone());
        let second = SyntaxService::with_factory(factory.clone());
        let third = SyntaxService::with_factory(factory);
        let size = MAX_IDLE_PROVIDER_BYTES / 2;
        assert!(local.retain_provider(&first.mailbox, Box::new(SizedProvider(size)), size).is_empty());
        assert!(local.retain_provider(&second.mailbox, Box::new(SizedProvider(size)), size).is_empty());
        let evicted = local.retain_provider(&third.mailbox, Box::new(SizedProvider(1)), 1);
        assert_eq!(evicted.len(), 1);
        assert!(local.take_provider(&first.mailbox).is_none());
        assert_eq!(local.take_provider(&second.mailbox).unwrap().retained_bytes(), size);
        assert_eq!(local.take_provider(&third.mailbox).unwrap().retained_bytes(), 1);
        drop(evicted);
        assert!(local.idle.lock().unwrap().is_empty());
        let oversized = MAX_IDLE_PROVIDER_BYTES + 1;
        let evicted = local.retain_provider(&first.mailbox, Box::new(SizedProvider(oversized)), oversized);
        assert_eq!(evicted.len(), 1);
        assert!(local.idle.lock().unwrap().is_empty());
    }

    #[test]
    fn suspended_provider_never_blocks_requests_supersession_or_publication() {
        let _registry = super::super::treesitter::package_registry_test_guard();
        let (started, receiver) = mpsc::channel();
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let factory: Factory = {
            let gate = gate.clone();
            Arc::new(move || {
                Box::new(Controlled {
                    started: started.clone(),
                    gate: gate.clone(),
                    first: true,
                })
            })
        };
        let mut service = SyntaxService::with_factory(factory);
        service.request(input(0), 0..100);
        assert_eq!(receiver.recv_timeout(Duration::from_secs(5)).unwrap(), 0);
        for revision in 1..100 {
            service.request(input(revision), 0..100);
        }
        assert_eq!(service.statistics.maximum_pending, 1);
        assert!(!service.poll(input(99).identity()));
        {
            let (lock, cv) = &*gate;
            *lock.lock().unwrap() = true;
            cv.notify_all();
        }
        assert_eq!(receiver.recv_timeout(Duration::from_secs(5)).unwrap(), 99);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !service.poll(input(99).identity()) {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(service.statistics.publications, 1);
        let requests = service.statistics.requests;
        service.request(input(99), 0..100);
        assert_eq!(requests, service.statistics.requests);
        assert_eq!(service.statistics.cache_hits, 1);
        assert!(service.runs(input(99).identity()).is_empty());
        assert!(service.retained_result_bytes() <= MAX_CACHED_RUN_BYTES);
    }

    #[test]
    fn filename_change_invalidates_configuration_and_rejects_late_results() {
        let _registry = super::super::treesitter::package_registry_test_guard();
        let mut service = SyntaxService::default();
        service.set_filename(Some("first.vim".into()));
        let snapshot = input(1);
        let request = SyntaxRequest {
            input: snapshot.clone(),
            configuration: service.configuration.clone(),
            range: 0..100,
        };
        let old = SyntaxResult::missing(&request, "old filename");
        let generation = service.configuration.generation;
        service.set_filename(Some("second.vim".into()));
        assert_ne!(service.configuration.generation, generation);
        service.mailbox.lock().unwrap().ready = Some(old);
        assert!(!service.poll(snapshot.identity()));
        assert_eq!(service.statistics.stale_rejections, 1);
        let generation = service.configuration.generation;
        service.set_filename(Some("second.vim".into()));
        assert_eq!(service.configuration.generation, generation);
    }

    #[test]
    fn factory_failure_does_not_strand_other_buffers() {
        let _registry = super::super::treesitter::package_registry_test_guard();
        let (started, receiver) = mpsc::channel();
        let gate = Arc::new((Mutex::new(true), Condvar::new()));
        let factory: Factory = Arc::new(move || {
            Box::new(Controlled {
                started: started.clone(),
                gate: gate.clone(),
                first: false,
            })
        });
        let mut service = SyntaxService::with_factory(factory);
        service.registry_generation = super::super::treesitter::package_registry_generation();
        let snapshot = input(4);
        service.request(snapshot.clone(), 0..100);
        assert_eq!(receiver.recv_timeout(Duration::from_secs(5)).unwrap(), 4);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !service.poll(snapshot.identity()) {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        let mut failed =
            SyntaxService::with_factory(Arc::new(|| panic!("deliberate provider factory failure")));
        failed.request(snapshot.clone(), 0..100);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !failed.poll(snapshot.identity()) {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(failed.diagnostics().contains("failed"));
        let requests = failed.statistics.requests;
        failed.request(snapshot, 0..100);
        assert_eq!(
            failed.statistics.requests, requests,
            "failed coverage is cached"
        );
        service.request(input(5), 0..100);
        assert_eq!(receiver.recv_timeout(Duration::from_secs(5)).unwrap(), 5);
    }

    #[test]
    fn closing_an_idle_provider_destroys_native_state_on_a_worker_outside_locks() {
        let _registry = super::super::treesitter::package_registry_test_guard();
        struct DropProbe {
            started: mpsc::Sender<()>,
            gate: Arc<(Mutex<bool>, Condvar)>,
        }
        impl SyntaxProvider for DropProbe {
            fn analyze(&mut self, r: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
                let mut result = SyntaxResult::missing(r, "");
                result.coverage = Coverage::Exact;
                result
            }
        }
        impl Drop for DropProbe {
            fn drop(&mut self) {
                assert!(
                    std::thread::current()
                        .name()
                        .unwrap_or("")
                        .starts_with("viem-syntax-"),
                    "native state dropped on foreground"
                );
                let _ = self.started.send(());
                let (lock, cv) = &*self.gate;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = cv.wait(released).unwrap();
                }
            }
        }
        let (started, receiver) = mpsc::channel();
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let factory: Factory = {
            let gate = gate.clone();
            Arc::new(move || {
                Box::new(DropProbe {
                    started: started.clone(),
                    gate: gate.clone(),
                })
            })
        };
        let mut service = SyntaxService::with_factory(factory);
        let snapshot = input(1);
        service.request(snapshot.clone(), 0..100);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !service.poll(snapshot.identity()) {
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        // A paint wait may outlive its service. Closing, not the final Arc
        // release, must retire the native provider without a periodic sweep.
        let detached_wait = {
            let slot = service.mailbox.lock().unwrap();
            SyntaxWait {
                mailbox: service.mailbox.clone(),
                request: SyntaxRequest { input: snapshot.clone(), configuration: service.configuration.clone(), range: 0..100 },
                cancellation: slot.cancellation.clone(),
                deadline: Instant::now() + SYNTAX_PRESENTATION_WAIT,
            }
        };
        drop(service); // Must return before the deliberately suspended native destructor.
        assert!(!detached_wait.wait_until(Instant::now() + SYNTAX_PRESENTATION_WAIT));
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        // Other parallel services may briefly hold this shared cache lock.
        // It must remain acquirable while this native destructor is suspended.
        let (acquired, acquisition) = mpsc::channel();
        let probe = std::thread::spawn(move || {
            let _idle = pool().idle.lock().unwrap();
            let _ = acquired.send(());
        });
        let lock_available = acquisition.recv_timeout(Duration::from_secs(5));
        {
            let (lock, cv) = &*gate;
            *lock.lock().unwrap() = true;
            cv.notify_all();
        }
        probe.join().unwrap();
        assert!(
            lock_available.is_ok(),
            "native destructor retained the cache lock"
        );
    }

    #[test]
    fn continuation_budget_survives_provider_eviction() {
        let _registry = super::super::treesitter::package_registry_test_guard();
        struct Yields {
            gate: Arc<(Mutex<bool>, Condvar)>,
        }
        impl SyntaxProvider for Yields {
            fn analyze(&mut self, r: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
                let (lock, cv) = &*self.gate;
                let mut ready = lock.lock().unwrap();
                while !*ready {
                    ready = cv.wait(ready).unwrap();
                }
                let mut result = SyntaxResult::missing(r, "pending");
                result.continuation = true;
                result
            }
        }
        // Queue more live sessions than the pool can retain before releasing
        // either worker. FIFO continuations then exercise real idle eviction.
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let created = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut services = Vec::new();
        let snapshot = input(1);
        for _ in 0..MAX_IDLE_PROVIDER_SESSIONS + WORKER_COUNT + 2 {
            let (gate, created) = (gate.clone(), created.clone());
            let mut service = SyntaxService::with_factory(Arc::new(move || {
                created.fetch_add(1, Ordering::SeqCst);
                Box::new(Yields { gate: gate.clone() })
            }));
            service.request(snapshot.clone(), 0..100);
            services.push(service);
        }
        {
            let (lock, cv) = &*gate;
            *lock.lock().unwrap() = true;
            cv.notify_all();
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            for service in &mut services {
                service.poll(snapshot.identity());
            }
            if services
                .iter()
                .all(|s| s.diagnostics().contains("continuation budget exhausted"))
            {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(
            created.load(Ordering::SeqCst) > services.len(),
            "evicted sessions must have been recreated"
        );
        for service in &services {
            let slot = service.mailbox.lock().unwrap();
            assert_eq!(slot.continuation_slices, MAX_CONTINUATION_SLICES);
            assert!(!slot.running);
        }
        assert!(pool().idle.lock().unwrap().len() <= MAX_IDLE_PROVIDER_SESSIONS);
    }

    #[test]
    fn failed_large_request_does_not_cover_smaller_regions() {
        let _registry = super::super::treesitter::package_registry_test_guard();
        // Like a whole-document query over its predicate budget: large regions
        // fail terminally while visible-sized regions succeed.
        struct Bounded(Arc<std::sync::atomic::AtomicUsize>);
        impl SyntaxProvider for Bounded {
            fn analyze(&mut self, r: &SyntaxRequest, _: &AtomicBool) -> SyntaxResult {
                self.0.fetch_add(1, Ordering::SeqCst);
                if r.range.len() > 1000 {
                    return SyntaxResult::missing(r, "over budget");
                }
                SyntaxResult {
                    input: r.input.identity(),
                    configuration: r.configuration.clone(),
                    range: r.range.clone(),
                    runs: vec![SyntaxRun {
                        range: r.range.start..r.range.start + 1,
                        name: crate::document::syntax::SyntaxStyleName("Keyword".into()),
                        origin: "test".into(),
                        priority: 0,
                    }],
                    coverage: Coverage::Exact,
                    diagnostics: Vec::new(),
                    continuation: false,
                }
            }
        }
        let analyzed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut service = SyntaxService::with_factory({
            let analyzed = analyzed.clone();
            Arc::new(move || Box::new(Bounded(analyzed.clone())))
        });
        let snapshot = input(1);
        let whole = 0..snapshot.byte_len();
        let finish = |service: &mut SyntaxService, range: std::ops::Range<usize>| {
            service.request(snapshot.clone(), range);
            let deadline = Instant::now() + Duration::from_secs(5);
            while !service.poll(snapshot.identity()) {
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
        };
        finish(&mut service, 0..100);
        finish(&mut service, whole.clone());
        assert_eq!(analyzed.load(Ordering::SeqCst), 2);
        // Neither the visible coverage nor the settled failure is requested
        // again, so they cannot evict each other in a loop.
        assert!(service.covers(&snapshot, 0..100));
        assert!(service.covers(&snapshot, whole.clone()));
        assert!(!service.covers(&snapshot, 1500..1600));
        finish(&mut service, 1500..1600);
        assert_eq!(analyzed.load(Ordering::SeqCst), 3);
        assert!(service.covers(&snapshot, 0..100));
        assert!(service.covers(&snapshot, whole));
        let starts = service.runs(snapshot.identity()).iter().map(|run| run.range.start).collect::<Vec<_>>();
        assert_eq!(starts, [0, 1500]);
    }
}
