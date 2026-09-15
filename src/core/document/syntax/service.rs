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
};

mod providers;
mod retention;
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
            vim_directory: if cfg!(target_os = "macos") {
                "/opt/homebrew/Cellar/macvim/9.1.1887/MacVim.app/Contents/Resources/vim/runtime/syntax".into()
            } else {
                String::new()
            },
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
    factory: Factory,
    pending: Option<SyntaxRequest>,
    active: Option<SyntaxRequest>,
    ready: Option<SyntaxResult>,
    cancellation: Arc<AtomicBool>,
    queued: bool,
    running: bool,
    closed: bool,
    continuation_key: Option<(SyntaxInputIdentity, SyntaxConfiguration)>,
    continuation_slices: usize,
}
type Mailbox = Arc<Mutex<Slot>>;
struct IdleProvider {
    mailbox: Weak<Mutex<Slot>>,
    provider: Box<dyn SyntaxProvider>,
    bytes: usize,
}
struct Pool {
    queue: Mutex<VecDeque<Weak<Mutex<Slot>>>>,
    available: Condvar,
    idle: Mutex<VecDeque<IdleProvider>>,
}
static POOL: OnceLock<Arc<Pool>> = OnceLock::new();
fn pool() -> &'static Arc<Pool> {
    POOL.get_or_init(|| {
        let pool = Arc::new(Pool {
            queue: Mutex::new(VecDeque::new()),
            available: Condvar::new(),
            idle: Mutex::new(VecDeque::new()),
        });
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
        let mut idle = self.idle.lock().unwrap_or_else(|e| e.into_inner());
        idle.push_back(IdleProvider {
            mailbox: Arc::downgrade(mailbox),
            provider,
            bytes,
        });
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
                let closed = idle[index].mailbox.strong_count() == 0;
                if closed {
                    retired.push(idle.remove(index).unwrap());
                }
            }
            retired
        };
        drop(retired);
    }
    fn enqueue(&self, mailbox: &Mailbox) -> bool {
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        if queue.len() >= MAX_QUEUE_BUFFERS {
            queue.retain(|slot| slot.strong_count() > 0);
        }
        if queue.len() >= MAX_QUEUE_BUFFERS {
            return false;
        }
        queue.push_back(Arc::downgrade(mailbox));
        self.available.notify_one();
        true
    }
    fn run(&self) {
        loop {
            self.retire_closed();
            let mailbox = {
                let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
                if queue.is_empty() {
                    queue = self
                        .available
                        .wait_timeout(queue, std::time::Duration::from_millis(100))
                        .unwrap_or_else(|e| e.into_inner())
                        .0;
                }
                queue.pop_front().and_then(|v| v.upgrade())
            };
            let Some(mailbox) = mailbox else {
                continue;
            };
            let job = {
                let mut slot = mailbox.lock().unwrap_or_else(|e| e.into_inner());
                slot.queued = false;
                if slot.closed {
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
            let mut slot = mailbox.lock().unwrap_or_else(|e| e.into_inner());
            slot.running = false;
            slot.active = None;
            if slot.closed {
                drop(slot);
                drop(provider);
                continue;
            }
            // Publish idle session ownership before admitting a continuation.
            // Another worker cannot take this mailbox before both are ready.
            let retired = provider.map(|provider| self.retain_provider(&mailbox, provider, provider_bytes)).unwrap_or_default();
            if !cancellation.load(Ordering::Acquire) {
                if result.continuation {
                    slot.continuation_slices += 1;
                    if slot.continuation_slices >= MAX_CONTINUATION_SLICES {
                        result.continuation = false;
                        result
                            .diagnostics
                            .push("Shared syntax continuation budget exhausted".into());
                    }
                }
                if result.continuation && slot.pending.is_none() {
                    slot.pending = Some(request);
                }
                slot.ready = Some(result);
            }
            if slot.pending.is_some() && !slot.queued {
                slot.queued = self.enqueue(&mailbox);
            }
            drop(slot);
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
pub struct SyntaxService {
    mailbox: Mailbox,
    pub configuration: SyntaxConfiguration,
    cache: VecDeque<SyntaxResult>,
    current: Option<SyntaxInputIdentity>,
    current_input: Option<SyntaxInputSnapshot>,
    /// Mapped appearance only; these runs cannot satisfy analysis requests.
    retained: Vec<SyntaxRun>,
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
    pub fn with_factory(factory: Factory) -> Self {
        Self {
            mailbox: Arc::new(Mutex::new(Slot {
                factory,
                pending: None,
                active: None,
                ready: None,
                cancellation: Arc::new(AtomicBool::new(false)),
                queued: false,
                running: false,
                closed: false,
                continuation_key: None,
                continuation_slices: 0,
            })),
            configuration: Default::default(),
            cache: VecDeque::new(),
            current: None,
            current_input: None,
            retained: Vec::new(),
            statistics: Default::default(),
            diagnostics: Vec::new(),
            registry_generation: 0,
        }
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
        self.current = None;
        self.current_input = None;
        self.retained.clear();
    }
    pub fn cancel(&mut self) {
        let mut slot = self.mailbox.lock().unwrap_or_else(|e| e.into_inner());
        slot.cancellation.store(true, Ordering::Release);
        slot.pending = None;
        slot.ready = None;
    }
    pub fn request(&mut self, input: SyntaxInputSnapshot, mut range: Range<usize>) {
        let registry = super::treesitter::package_registry_generation();
        if registry != self.registry_generation {
            self.registry_generation = registry;
            self.configuration.registry_generation = registry;
            self.invalidate_configuration();
        }
        if range.start > range.end || range.end > input.byte_len() {
            return;
        }
        range.end = range.end.min(range.start.saturating_add(MAX_REGION_BYTES));
        while range.end > range.start
            && !input
                .text_tree()
                .is_char_boundary(range.end)
                .unwrap_or(false)
        {
            range.end -= 1;
        }
        if self.current != Some(input.identity()) {
            self.rebase_input(input.clone(), None);
        }
        if self.cache.iter().any(|r| {
            r.input == input.identity()
                && r.configuration == self.configuration
                && r.range.start <= range.start
                && r.range.end >= range.end
        }) {
            self.statistics.cache_hits += 1;
            return;
        }
        let request = SyntaxRequest {
            input,
            configuration: self.configuration.clone(),
            range,
        };
        let mut slot = self.mailbox.lock().unwrap_or_else(|e| e.into_inner());
        if slot.pending.as_ref().is_some_and(|r| r.same(&request)) {
            // A full shared queue leaves the coalesced request in its mailbox.
            // A later frame must retry admission instead of stranding it.
            if !slot.running && !slot.queued {
                slot.queued = pool().enqueue(&self.mailbox);
            }
            return;
        }
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
        if !slot.running && !slot.queued {
            slot.queued = pool().enqueue(&self.mailbox);
        }
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
        let appearance_changed = !self.cache.iter().any(|old| {
            old.input == result.input
                && old.configuration == result.configuration
                && old.range == result.range
                && old.coverage == result.coverage
                && old.runs == result.runs
        });
        self.replace_presentation_coverage(&result);
        self.cache.push_back(result);
        self.bound_presentation_cache();
        if appearance_changed {
            self.statistics.publications += 1;
        }
        appearance_changed
    }
    pub fn runs(&self, input: SyntaxInputIdentity) -> Vec<SyntaxRun> {
        let mut runs = self
            .cache
            .iter()
            .filter(|r| r.input == input)
            .flat_map(|r| r.runs.iter().cloned())
            .collect::<Vec<_>>();
        if self.current == Some(input) {
            runs.extend(self.retained.iter().cloned());
        }
        runs.sort_by_key(|r| r.range.start);
        runs
    }
    pub fn diagnostics(&self) -> String {
        self.diagnostics.join("\n")
    }
    pub fn retained_result_bytes(&self) -> usize {
        self.cache.iter().map(SyntaxResult::bytes).sum::<usize>()
            + self.retained.iter().map(retention::run_bytes).sum::<usize>()
    }
}
impl Drop for SyntaxService {
    fn drop(&mut self) {
        let mut slot = self.mailbox.lock().unwrap_or_else(|e| e.into_inner());
        slot.closed = true;
        slot.cancellation.store(true, Ordering::Release);
        slot.pending = None;
        slot.ready = None;
        if let Some(pool) = POOL.get() {
            pool.available.notify_all();
        }
    }
}

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
    fn input(revision: u64) -> SyntaxInputSnapshot {
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
        let local = Pool { queue: Mutex::new(VecDeque::new()), available: Condvar::new(),
            idle: Mutex::new(VecDeque::new()) };
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
    fn queue_admission_retry_and_factory_failure_do_not_strand_other_buffers() {
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
        // Exact mailbox state left after an admission attempt meets a full queue.
        service.mailbox.lock().unwrap().pending = Some(SyntaxRequest {
            input: snapshot.clone(),
            configuration: service.configuration.clone(),
            range: 0..100,
        });
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
        drop(service); // Must return before the deliberately suspended native destructor.
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        // The destructor cannot retain the pool's cache lock.
        assert!(pool().idle.try_lock().is_ok());
        {
            let (lock, cv) = &*gate;
            *lock.lock().unwrap() = true;
            cv.notify_all();
        }
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
}
