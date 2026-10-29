use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

type Job = Box<dyn FnOnce() + Send + 'static>;
const MAX_QUEUED_JOBS: usize = 64;

pub struct WorkerPool {
    sender: Option<SyncSender<Job>>,
    workers: Vec<JoinHandle<()>>,
    active_jobs: Arc<AtomicUsize>,
}

impl WorkerPool {
    pub fn new(num_threads: usize) -> Self {
        assert!(num_threads > 0, "a worker pool needs at least one worker");
        let (sender, receiver) = sync_channel::<Job>(MAX_QUEUED_JOBS);
        let receiver = Arc::new(Mutex::new(receiver));
        let active_jobs = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::with_capacity(num_threads);

        for id in 0..num_threads {
            let rx = receiver.clone();
            let active = active_jobs.clone();
            let handle = thread::Builder::new()
                .name(format!("wocli-worker-{id}"))
                .spawn(move || loop {
                    let job = {
                        let lock = match rx.lock() {
                            Ok(guard) => guard,
                            Err(_) => break, // Poisoned
                        };
                        match lock.recv() {
                            Ok(j) => j,
                            Err(_) => break, // Channel closed
                        }
                    };
                    active.fetch_add(1, Ordering::Relaxed);
                    // Keep accounting and capacity intact in unwind builds.
                    // Release builds use panic=abort and terminate on panic.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                    active.fetch_sub(1, Ordering::Relaxed);
                })
                .expect("Failed to spawn worker thread");
            workers.push(handle);
        }

        Self {
            sender: Some(sender),
            workers,
            active_jobs,
        }
    }

    pub fn execute(&self, f: impl FnOnce() + Send + 'static) -> bool {
        if let Some(ref sender) = self.sender {
            sender.try_send(Box::new(f)).is_ok()
        } else {
            false
        }
    }

    pub fn active_count(&self) -> usize {
        self.active_jobs.load(Ordering::Relaxed)
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        drop(self.sender.take());
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

pub struct ConnectionLimiter {
    current: Arc<AtomicUsize>,
    max_connections: usize,
}

pub struct ConnectionGuard {
    counter: Arc<AtomicUsize>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

impl ConnectionLimiter {
    pub fn new(max_connections: usize) -> Self {
        Self {
            current: Arc::new(AtomicUsize::new(0)),
            max_connections,
        }
    }

    pub fn acquire(&self) -> Option<ConnectionGuard> {
        let mut count = self.current.load(Ordering::SeqCst);
        loop {
            if count >= self.max_connections {
                return None;
            }
            match self.current.compare_exchange_weak(
                count,
                count + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => {
                    return Some(ConnectionGuard {
                        counter: self.current.clone(),
                    });
                }
                Err(actual) => count = actual,
            }
        }
    }

    pub fn current(&self) -> usize {
        self.current.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    #[test]
    fn test_worker_pool_executes_tasks() {
        let pool = WorkerPool::new(2);
        let done = Arc::new(AtomicBool::new(false));
        let done_clone = done.clone();
        assert!(pool.execute(move || {
            done_clone.store(true, Ordering::SeqCst);
        }));
        thread::sleep(Duration::from_millis(50));
        assert!(done.load(Ordering::SeqCst));
    }

    #[test]
    fn saturated_pool_rejects_work_without_blocking_and_recovers_after_drain() {
        let pool = WorkerPool::new(1);
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        assert!(pool.execute(move || {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        }));
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let completed = Arc::new(AtomicUsize::new(0));
        for _ in 0..MAX_QUEUED_JOBS {
            let completed = completed.clone();
            assert!(pool.execute(move || {
                completed.fetch_add(1, Ordering::Relaxed);
            }));
        }
        let rejected_capture = Arc::new(());
        let captured = rejected_capture.clone();
        let start = std::time::Instant::now();
        assert!(!pool.execute(move || drop(captured)));
        assert!(start.elapsed() < Duration::from_millis(100));
        assert_eq!(Arc::strong_count(&rejected_capture), 1);
        release_tx.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while completed.load(Ordering::Relaxed) != MAX_QUEUED_JOBS {
            assert!(std::time::Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(pool.execute(move || tx.send(()).unwrap()));
        rx.recv_timeout(Duration::from_secs(1)).unwrap();
        drop(pool);
        assert_eq!(completed.load(Ordering::Relaxed), MAX_QUEUED_JOBS);
    }

    #[test]
    #[cfg(panic = "unwind")]
    fn panicking_job_does_not_retire_a_worker_or_leak_active_count() {
        let pool = WorkerPool::new(1);
        assert!(pool.execute(|| panic!("test task panic")));
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(pool.execute(move || tx.send(()).unwrap()));
        rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let active = pool.active_jobs.clone();
        drop(pool);
        assert_eq!(active.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_connection_limiter_caps_concurrent_connections() {
        let limiter = ConnectionLimiter::new(2);
        let g1 = limiter.acquire();
        assert!(g1.is_some());
        assert_eq!(limiter.current(), 1);

        let g2 = limiter.acquire();
        assert!(g2.is_some());
        assert_eq!(limiter.current(), 2);

        // Third should be rejected
        let g3 = limiter.acquire();
        assert!(g3.is_none());

        drop(g1);
        assert_eq!(limiter.current(), 1);

        let g4 = limiter.acquire();
        assert!(g4.is_some());
        assert_eq!(limiter.current(), 2);
    }
}
