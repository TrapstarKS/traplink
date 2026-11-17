use std::sync::{mpsc, Mutex, OnceLock};

type Job = Box<dyn FnOnce() + Send>;

static WORKER: OnceLock<Option<Mutex<mpsc::Sender<Job>>>> = OnceLock::new();

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
    worker
        .lock()
        .map_err(|_| "HID access is unavailable on this computer.")?
        .send(Box::new(move || {
            let _ = done.send(job());
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
}
