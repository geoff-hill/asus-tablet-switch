use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, Device, EventSummary, KeyCode, SwitchCode, SwitchEvent};
use signal_hook::consts::{SIGINT, SIGTERM};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const SOURCE_DEVICE_NAME: &str = "Asus WMI hotkeys";
const LID_DEVICE_NAME: &str = "Lid Switch";
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

    fn recover_laptop_mode(&mut self) -> bool {
        let changed = self.mode != Mode::Laptop;
        self.mode = Mode::Laptop;
        self.last_transition = None;
        changed
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

fn is_lid_open(event: &evdev::InputEvent) -> bool {
    matches!(
        event.destructure(),
        EventSummary::Switch(_, SwitchCode::SW_LID, 0)
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

fn discover_device(name: &str) -> Option<(std::path::PathBuf, Device)> {
    for (path, device) in evdev::enumerate() {
        if device.name() == Some(name) {
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

    // There is no absolute hinge state to query at startup, so assume laptop. A later lid-open
    // event provides an absolute recovery point if hinge-event parity becomes incorrect.
    let initial_mode = Mode::Laptop;
    let mut filter = TransitionFilter::new(initial_mode, DEBOUNCE_INTERVAL);
    emit_mode(&mut virtual_device, initial_mode)?;
    eprintln!("initial mode: laptop (SW_TABLET_MODE=0; assumed)");

    let started = Instant::now();
    let mut retry_delay = RETRY_MIN;

    while !terminate.load(Ordering::Relaxed) {
        let Some((path, mut source)) = discover_device(SOURCE_DEVICE_NAME) else {
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
        let mut lid_source: Option<(std::path::PathBuf, Device)> = None;
        let mut next_lid_discovery = Instant::now();

        'connected: while !terminate.load(Ordering::Relaxed) {
            let source_idle = match source.fetch_events() {
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
                    false
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => true,
                Err(error) => {
                    eprintln!(
                        "source device {} disconnected or failed: {error}; rediscovering",
                        path.display()
                    );
                    wait_interruptibly(&terminate, retry_delay);
                    retry_delay = (retry_delay * 2).min(RETRY_MAX);
                    break 'connected;
                }
            };

            if lid_source.is_none() && Instant::now() >= next_lid_discovery {
                match discover_device(LID_DEVICE_NAME) {
                    Some((lid_path, lid)) => match lid.set_nonblocking(true) {
                        Ok(()) => {
                            eprintln!(
                                "selected recovery device: {} ({LID_DEVICE_NAME})",
                                lid_path.display()
                            );
                            lid_source = Some((lid_path, lid));
                        }
                        Err(error) => {
                            eprintln!(
                                "could not use recovery device {}: {error}; retrying",
                                lid_path.display()
                            );
                            next_lid_discovery = Instant::now() + RETRY_MAX;
                        }
                    },
                    None => {
                        eprintln!(
                            "recovery device {LID_DEVICE_NAME:?} not found; retrying in {:.2}s",
                            RETRY_MAX.as_secs_f32()
                        );
                        next_lid_discovery = Instant::now() + RETRY_MAX;
                    }
                }
            }

            let mut failed_lid = None;
            if let Some((lid_path, lid)) = lid_source.as_mut() {
                match lid.fetch_events() {
                    Ok(events) => {
                        for event in events {
                            if is_lid_open(&event) {
                                if filter.recover_laptop_mode() {
                                    emit_mode(&mut virtual_device, Mode::Laptop)?;
                                    eprintln!(
                                        "lid opened: recovered laptop mode (SW_TABLET_MODE=0)"
                                    );
                                } else {
                                    eprintln!("lid opened: laptop mode already active");
                                }
                            }
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => failed_lid = Some((lid_path.clone(), error)),
                }
            }

            if let Some((lid_path, error)) = failed_lid {
                eprintln!(
                    "recovery device {} disconnected or failed: {error}; rediscovering",
                    lid_path.display()
                );
                lid_source = None;
                next_lid_discovery = Instant::now() + RETRY_MIN;
            }

            if source_idle {
                wait_interruptibly(&terminate, POLL_INTERVAL);
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

    #[test]
    fn lid_open_recovers_laptop_mode() {
        let mut filter = filter();
        filter.observe(true, Duration::ZERO);

        assert!(filter.recover_laptop_mode());
        assert_eq!(filter.mode, Mode::Laptop);
    }

    #[test]
    fn lid_open_clears_hinge_debounce() {
        let mut filter = filter();
        filter.observe(true, Duration::ZERO);
        filter.recover_laptop_mode();

        assert_eq!(
            filter.observe(true, Duration::from_millis(50)),
            FilterResult::Transition(Mode::Tablet)
        );
    }

    #[test]
    fn lid_open_in_laptop_mode_needs_no_transition() {
        let mut filter = filter();
        assert!(!filter.recover_laptop_mode());
        assert_eq!(filter.mode, Mode::Laptop);
    }

    #[test]
    fn lid_open_switch_event_is_recognized() {
        let event = SwitchEvent::new(SwitchCode::SW_LID, 0).into();
        assert!(is_lid_open(&event));
    }

    #[test]
    fn lid_close_switch_event_is_not_a_recovery() {
        let event = SwitchEvent::new(SwitchCode::SW_LID, 1).into();
        assert!(!is_lid_open(&event));
    }
}
