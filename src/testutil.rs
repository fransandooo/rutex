//! GTK can only be used from the thread that initialized it, and tests run in
//! parallel threads: every test that needs widgets runs its body here, on one
//! shared GTK thread.

use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};

type Job = Box<dyn FnOnce() + Send>;

/// Runs `f` on the GTK thread and waits; a panic inside fails the caller.
/// Returns without running `f` when there is no display.
pub fn with_gtk(f: impl FnOnce() + Send + 'static) {
    static JOBS: OnceLock<Option<Mutex<Sender<(Job, Sender<std::thread::Result<()>>)>>>> = OnceLock::new();
    let jobs = JOBS.get_or_init(|| {
        let (tx, rx) = channel::<(Job, Sender<std::thread::Result<()>>)>();
        let (ready_tx, ready_rx) = channel();
        std::thread::spawn(move || {
            let ok = gtk::init().is_ok() && adw::init().is_ok();
            let _ = ready_tx.send(ok);
            if !ok {
                return;
            }
            for (job, done) in rx {
                let _ = done.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)));
            }
        });
        ready_rx.recv().unwrap_or(false).then(|| Mutex::new(tx))
    });
    let Some(jobs) = jobs else { return };
    let (done_tx, done_rx) = channel();
    jobs.lock().unwrap().send((Box::new(f), done_tx)).unwrap();
    if let Err(panic) = done_rx.recv().unwrap() {
        std::panic::resume_unwind(panic);
    }
}
