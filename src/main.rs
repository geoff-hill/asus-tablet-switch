use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, Device, EventSummary, KeyCode, SwitchCode, SwitchEvent};
use signal_hook::consts::{SIGINT, SIGTERM};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const SOURCE_DEVICE_NAME: &str = "Asus WMI hotkeys";
const VIRTUAL_DEVICE_NAME: &str = "ASUS Virtual Tablet Mode Switch";
const DEBOUNCE_INTERVAL: Duration = Duration::from_millis(750);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const RETRY_MIN: Duration = Duration::from_millis(250);
const RETRY_MAX: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Laptop,
    Tablet,
}

impl Mode {
    fn toggled(self) -> Self {
        match self {
            Self::Laptop => Self::Tablet,
            Self::Tablet => Self::Laptop,
        }
    }

    fn switch_value(self) -> i32 {
        match self {
            Self::Laptop => 0,
            Self::Tablet => 1,
        }
    }
}

#[derive(Debug)]
struct TransitionFilter {
    mode: Mode,
    last_transition: Option<Duration>,
    debounce_interval: Duration,
}

impl TransitionFilter {
    fn new(initial_mode: Mode, debounce_interval: Duration) -> Self {
        Self {
            mode: initial_mode,
            last_transition: None,
            debounce_interval,
        }
    }

    fn observe(&mut self, is_prog2_press: bool, now: Duration) -> FilterResult {
        if !is_prog2_press {
            return FilterResult::Unrelated;
        }

        if self
            .last_transition
            .is_some_and(|last| now.saturating_sub(last) < self.debounce_interval)
        {
            return FilterResult::Duplicate;
        }

        self.mode = self.mode.toggled();
        self.last_transition = Some(now);
        FilterResult::Transition(self.mode)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FilterResult {
    Unrelated,
    Duplicate,
    Transition(Mode),
}

fn is_prog2_press(event: &evdev::InputEvent) -> bool {
    matches!(
        event.destructure(),
        EventSummary::Key(_, KeyCode::KEY_PROG2, 1)
    )
}

fn create_virtual_device() -> io::Result<VirtualDevice> {
    let mut switches = AttributeSet::<SwitchCode>::new();
    switches.insert(SwitchCode::SW_TABLET_MODE);

    VirtualDevice::builder()?
        .name(VIRTUAL_DEVICE_NAME)
        .with_switches(&switches)?
        .build()
}

fn emit_mode(device: &mut VirtualDevice, mode: Mode) -> io::Result<()> {
    device.emit(&[SwitchEvent::new(SwitchCode::SW_TABLET_MODE, mode.switch_value()).into()])
}

fn discover_source() -> Option<(std::path::PathBuf, Device)> {
    for (path, device) in evdev::enumerate() {
        if device.name() == Some(SOURCE_DEVICE_NAME) {
            return Some((path, device));
        }
    }
    None
}

fn wait_interruptibly(terminate: &AtomicBool, duration: Duration) {
    let deadline = Instant::now() + duration;
    while !terminate.load(Ordering::Relaxed) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        thread::sleep(remaining.min(POLL_INTERVAL));
    }
}

fn run(terminate: Arc<AtomicBool>) -> io::Result<()> {
    eprintln!("starting {VIRTUAL_DEVICE_NAME}");

    let mut virtual_device = create_virtual_device()?;

    // Deliberate initial-state policy: there is no absolute hinge state to query, so assume laptop.
    // Keeping this in one place makes a future state-recovery strategy straightforward to add.
    let initial_mode = Mode::Laptop;
    let mut filter = TransitionFilter::new(initial_mode, DEBOUNCE_INTERVAL);
    emit_mode(&mut virtual_device, initial_mode)?;
    eprintln!("initial mode: laptop (SW_TABLET_MODE=0; assumed)");

    let started = Instant::now();
    let mut retry_delay = RETRY_MIN;

    while !terminate.load(Ordering::Relaxed) {
        let Some((path, mut source)) = discover_source() else {
            eprintln!(
                "source device {SOURCE_DEVICE_NAME:?} not found; retrying in {:.2}s",
                retry_delay.as_secs_f32()
            );
            wait_interruptibly(&terminate, retry_delay);
            retry_delay = (retry_delay * 2).min(RETRY_MAX);
            continue;
        };

        eprintln!(
            "selected source device: {} ({SOURCE_DEVICE_NAME})",
            path.display()
        );
        source.set_nonblocking(true)?;
        retry_delay = RETRY_MIN;

        'connected: while !terminate.load(Ordering::Relaxed) {
            match source.fetch_events() {
                Ok(events) => {
                    for event in events {
                        match filter.observe(is_prog2_press(&event), started.elapsed()) {
                            FilterResult::Transition(mode) => {
                                emit_mode(&mut virtual_device, mode)?;
                                match mode {
                                    Mode::Laptop => {
                                        eprintln!("mode transition: laptop (SW_TABLET_MODE=0)")
                                    }
                                    Mode::Tablet => {
                                        eprintln!("mode transition: tablet (SW_TABLET_MODE=1)")
                                    }
                                }
                            }
                            FilterResult::Duplicate => {
                                eprintln!("ignored duplicate KEY_PROG2 press")
                            }
                            FilterResult::Unrelated => {}
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    wait_interruptibly(&terminate, POLL_INTERVAL);
                }
                Err(error) => {
                    eprintln!(
                        "source device {} disconnected or failed: {error}; rediscovering",
                        path.display()
                    );
                    wait_interruptibly(&terminate, retry_delay);
                    retry_delay = (retry_delay * 2).min(RETRY_MAX);
                    break 'connected;
                }
            }
        }
    }

    eprintln!("termination requested; exiting cleanly");
    Ok(())
}

fn main() -> io::Result<()> {
    let terminate = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGTERM, Arc::clone(&terminate))?;
    signal_hook::flag::register(SIGINT, Arc::clone(&terminate))?;
    run(terminate)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter() -> TransitionFilter {
        TransitionFilter::new(Mode::Laptop, Duration::from_millis(750))
    }

    #[test]
    fn first_prog2_press_toggles_state() {
        let mut filter = filter();
        assert_eq!(
            filter.observe(true, Duration::ZERO),
            FilterResult::Transition(Mode::Tablet)
        );
    }

    #[test]
    fn duplicate_press_inside_debounce_interval_is_ignored() {
        let mut filter = filter();
        filter.observe(true, Duration::ZERO);
        assert_eq!(
            filter.observe(true, Duration::from_millis(50)),
            FilterResult::Duplicate
        );
        assert_eq!(filter.mode, Mode::Tablet);
    }

    #[test]
    fn next_burst_after_debounce_interval_toggles_state() {
        let mut filter = filter();
        filter.observe(true, Duration::ZERO);
        assert_eq!(
            filter.observe(true, Duration::from_millis(750)),
            FilterResult::Transition(Mode::Laptop)
        );
    }

    #[test]
    fn unrelated_event_does_nothing() {
        let mut filter = filter();
        assert_eq!(
            filter.observe(false, Duration::ZERO),
            FilterResult::Unrelated
        );
        assert_eq!(filter.mode, Mode::Laptop);
    }
}
