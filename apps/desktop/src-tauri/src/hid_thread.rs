use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc, Mutex, OnceLock,
};

type Job = Box<dyn FnOnce() + Send>;

static WORKER: OnceLock<Option<Mutex<mpsc::Sender<Job>>>> = OnceLock::new();
static PENDING: AtomicUsize = AtomicUsize::new(0);

/// Counts a job from submission until it finishes (or is dropped unrun).
struct Pending;
impl Pending {
    fn start() -> Self {
        PENDING.fetch_add(1, Ordering::AcqRel);
        Self
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        PENDING.fetch_sub(1, Ordering::AcqRel);
    }
}

/// True while any HID job is queued or running, so callers that can use
/// recent data avoid waiting behind slow HID++ requests.
pub fn busy() -> bool {
    PENDING.load(Ordering::Acquire) > 0
}

pub fn run<R: Send + 'static>(job: impl FnOnce() -> R + Send + 'static) -> Result<R, String> {
    let worker = WORKER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("traplink-hid".into())
            .spawn(move || {
                for job in receiver {
                    job();
                }
            })
            .ok()
            .map(|_| Mutex::new(sender))
    });
    let worker = worker
        .as_ref()
        .ok_or("HID access is unavailable on this computer.")?;
    let (done, wait) = mpsc::sync_channel(1);
    let pending = Pending::start();
    worker
        .lock()
        .map_err(|_| "HID access is unavailable on this computer.")?
        .send(Box::new(move || {
            let result = job();
            drop(pending);
            let _ = done.send(result);
        }))
        .map_err(|_| "HID access is unavailable on this computer.".to_string())?;
    wait.recv()
        .map_err(|_| "HID access stopped unexpectedly. Try again.".to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn jobs_run_on_one_long_lived_thread() {
        let first = super::run(|| std::thread::current().id()).unwrap();
        let second = super::run(|| std::thread::current().id()).unwrap();
        assert_eq!(first, second);
        assert_ne!(first, std::thread::current().id());
        assert_eq!(
            super::run(|| std::thread::current().name().map(String::from)).unwrap(),
            Some("traplink-hid".into())
        );
    }

    #[test]
    fn busy_while_a_job_is_queued_or_running() {
        let (release, hold) = std::sync::mpsc::channel::<()>();
        let job = std::thread::spawn(move || super::run(move || hold.recv()));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !super::busy() && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(super::busy());
        release.send(()).unwrap();
        job.join().unwrap().unwrap().unwrap();
    }
}
