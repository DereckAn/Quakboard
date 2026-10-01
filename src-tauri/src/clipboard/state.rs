use std::sync::{
    atomic::{AtomicBool, AtomicI32, Ordering},
    Mutex,
};
use std::time::{Duration, Instant};

// AtomicI32 - a thread-safe integer. We us i32 (signed) so we can use -1 as "not set" value.
// AtomicUsize - a thread-safe unsigned integer. We use usize for counting events to skip.

// lazy_static! - creates global variables that are initialized once at first use.
lazy_static::lazy_static! {
    static ref SKIP_EVENTS: Mutex<SkipWindow> = Mutex::new(SkipWindow::default());
    static ref PREVIOUS_APP_PID: AtomicI32 = AtomicI32::new(-1);
    // When the capture-folder watcher owns screenshots, the clipboard monitor
    // skips saving raw screenshot bytes so we don't get a duplicate entry.
    static ref SUPPRESS_SCREENSHOT_BYTES: AtomicBool = AtomicBool::new(false);
    // Set while one of our own dialogs is open: the window loses focus to it
    // and must not auto-hide, or the dialog ends up orphaned behind it.
    static ref KEEP_WINDOW_OPEN: AtomicBool = AtomicBool::new(false);
}

/// Whether the window should stay up despite losing focus (a dialog of ours
/// has it). See `KeepWindowOpen`.
pub fn should_keep_window_open() -> bool {
    KEEP_WINDOW_OPEN.load(Ordering::SeqCst)
}

/// Keeps the window from auto-hiding while alive; resets however the
/// dialog ends (chosen, cancelled, or an error).
pub struct KeepWindowOpen;

impl KeepWindowOpen {
    pub fn new() -> Self {
        KEEP_WINDOW_OPEN.store(true, Ordering::SeqCst);
        KeepWindowOpen
    }
}

impl Drop for KeepWindowOpen {
    fn drop(&mut self) {
        KEEP_WINDOW_OPEN.store(false, Ordering::SeqCst);
    }
}

/// Enable/disable suppression of raw clipboard screenshot bytes.
/// Set to `true` when the folder watcher is active (it stores pointers instead).
pub fn set_suppress_screenshot_bytes(suppress: bool) {
    SUPPRESS_SCREENSHOT_BYTES.store(suppress, Ordering::SeqCst);
}

/// True when the clipboard monitor should skip saving raw screenshot bytes.
pub fn should_suppress_screenshot_bytes() -> bool {
    SUPPRESS_SCREENSHOT_BYTES.load(Ordering::SeqCst)
}

pub fn store_previous_app_pid(pid: i32) {
    PREVIOUS_APP_PID.store(pid, Ordering::SeqCst);
    // Syntax note: Ordering::SeqCst means "sequentially consistent" —
    // the strictest memory ordering, guarantees all threads see the
    // same value. Safe default for our case.
}

// Take_previous_app_pid - reads the PID and resets it to -1 (so it can only be used once)
// .swap() - Automatically replaces the value and returns the old one
pub fn take_previous_app_pid() -> i32 {
    PREVIOUS_APP_PID.swap(-1, Ordering::SeqCst)
}

/// How long a request to ignore our own clipboard writes stays valid. The
/// events it's meant for arrive within milliseconds; the expiry stops a wrong
/// count from swallowing the user's next real copy (see `SkipWindow`).
const SKIP_WINDOW: Duration = Duration::from_secs(2);

/// Ask the monitor to ignore the next `count` clipboard changes, which our
/// own write is about to cause.
pub fn request_skip_events(count: usize) {
    lock_skips().request(count, Instant::now());
}

pub fn take_skip_event() -> bool {
    lock_skips().take(Instant::now())
}

fn lock_skips() -> std::sync::MutexGuard<'static, SkipWindow> {
    SKIP_EVENTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Pending skips and when they lapse. Without the expiry, a write that fires
/// fewer events than requested (macOS fires one where Wayland fires two)
/// left a skip behind that silently ignored the user's next copy.
#[derive(Default)]
struct SkipWindow {
    count: usize,
    until: Option<Instant>,
}

impl SkipWindow {
    fn request(&mut self, count: usize, now: Instant) {
        if count == 0 {
            return;
        }
        if !self.is_open(now) {
            self.count = 0;
        }
        self.count += count;
        self.until = Some(now + SKIP_WINDOW);
    }

    fn take(&mut self, now: Instant) -> bool {
        if self.is_open(now) && self.count > 0 {
            self.count -= 1;
            return true;
        }
        self.count = 0;
        self.until = None;
        false
    }

    fn is_open(&self, now: Instant) -> bool {
        self.until.is_some_and(|until| now < until)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_previous_app_pid() {
        store_previous_app_pid(12345);
        assert_eq!(take_previous_app_pid(), 12345);
    }

    #[test]
    fn test_take_previous_app_pid() {
        store_previous_app_pid(12345);
        assert_eq!(take_previous_app_pid(), 12345);
        assert_eq!(take_previous_app_pid(), -1);
    }

    fn window_with(count: usize, now: Instant) -> SkipWindow {
        let mut window = SkipWindow::default();
        window.request(count, now);
        window
    }

    #[test]
    fn window_stays_open_only_while_the_guard_lives() {
        let during = {
            let _guard = KeepWindowOpen::new();
            should_keep_window_open()
        };
        assert_eq!((during, should_keep_window_open()), (true, false));
    }

    #[test]
    fn requested_skip_is_taken_once() {
        let now = Instant::now();
        let mut window = window_with(1, now);
        assert_eq!((window.take(now), window.take(now)), (true, false));
    }

    #[test]
    fn requested_skips_add_up() {
        let now = Instant::now();
        let mut window = window_with(2, now);
        assert_eq!(
            (window.take(now), window.take(now), window.take(now)),
            (true, true, false)
        );
    }

    #[test]
    fn leftover_skip_does_not_swallow_a_later_copy() {
        // Asked for 2, only 1 event came (macOS): the user's copy 5 s later
        // must not be ignored.
        let now = Instant::now();
        let mut window = window_with(2, now);
        window.take(now);

        assert!(!window.take(now + Duration::from_secs(5)));
    }

    #[test]
    fn expired_skips_do_not_carry_into_a_new_request() {
        let now = Instant::now();
        let later = now + Duration::from_secs(5);
        let mut window = window_with(3, now);
        window.request(1, later);

        assert_eq!((window.take(later), window.take(later)), (true, false));
    }

    #[test]
    fn requesting_zero_skips_nothing() {
        let now = Instant::now();
        assert!(!window_with(0, now).take(now));
    }
}
