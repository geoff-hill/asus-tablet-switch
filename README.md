# ASUS tablet switch daemon

`asus-tablet-switch` translates the observed ASUS ProArt PX13 hinge hotkey burst into
a virtual Linux `SW_TABLET_MODE` switch. It finds the evdev source by the exact name
`Asus WMI hotkeys`; it never relies on an `eventN` number and does not grab the source.

Each `KEY_PROG2` press toggles an in-memory laptop/tablet state. A second press less
than 750 ms later is logged and ignored, collapsing the firmware's two-press/four-event
burst into one transition. The daemon creates a uinput device named
`ASUS Virtual Tablet Mode Switch`, publishes an initial `SW_TABLET_MODE=0`, and then
publishes `1` for tablet and `0` for laptop. Source disconnections cause name-based
rediscovery with retry delays from 250 ms up to 5 seconds. `SIGINT` and `SIGTERM` are
checked at most every 50 ms.

## Important limitation

The ASUS hotkey is a toggle, not an absolute hinge-state report. The daemon deliberately
starts in laptop mode; that policy is isolated in `run()` so it can later be replaced
with real state recovery. Starting the daemon while folded, restarting it while folded,
or missing a complete hinge event can invert its state until the next restart/correction.
The 750 ms debounce interval also assumes two genuine hinge crossings will not occur
within that window.

## Build and automated tests

Use stable Rust:

```console
cargo build --release
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

Runtime tools used below are available on Fedora as `evtest` and `libinput-utils`:

```console
sudo dnf install evtest libinput-utils
```

## Manual test first (do not enable the service yet)

The source and uinput nodes require privilege before the dedicated service account and
udev rule are installed, so use `sudo` only for this initial test. The daemon itself
does not need network access.

In terminal A:

```console
sudo modprobe uinput
sudo ./target/release/asus-tablet-switch
```

Startup should identify a dynamically discovered `/dev/input/eventN` and log the
assumed laptop state. In terminal B, use either tool and select/watch
`ASUS Virtual Tablet Mode Switch`:

```console
sudo libinput debug-events
```

or:

```console
sudo evtest
```

Perform this sequence:

1. Confirm the initial virtual state is laptop mode, `SW_TABLET_MODE=0`.
2. Fold just past flat. Confirm exactly one virtual `SW_TABLET_MODE=1`; terminal A
   should log one transition and one ignored duplicate `KEY_PROG2` press.
3. Rotate the folded machine and check whether GNOME now exposes or performs
   auto-rotation.
4. Unfold. Confirm exactly one virtual `SW_TABLET_MODE=0`, and confirm the physical
   keyboard and touchpad work again.
5. Stop terminal A with Ctrl-C and inspect its complete stderr output. It should log
   clean termination. For a later service run, inspect logs with:

```console
sudo journalctl -u asus-tablet-switch.service -b
```

## Install and enable only after the manual test passes

The system service uses a dedicated static user/group. The udev rule grants that group
read access only to an event node whose device name is exactly `Asus WMI hotkeys`, plus
read/write access to `/dev/uinput`. It does not globally change input-device permissions.
The service's device cgroup additionally allows read-only access to the input character
device class and read/write access to `/dev/uinput`. Unix ownership remains the narrower
gate. No Linux capabilities or root privileges remain in the running process.

```console
sudo useradd --system --user-group --home-dir / --shell /usr/sbin/nologin asus-tablet-switch
sudo install -Dm0755 target/release/asus-tablet-switch /usr/local/sbin/asus-tablet-switch
sudo install -Dm0644 packaging/asus-tablet-switch.service /etc/systemd/system/asus-tablet-switch.service
sudo install -Dm0644 packaging/99-asus-tablet-switch.rules /etc/udev/rules.d/99-asus-tablet-switch.rules
sudo install -Dm0644 packaging/asus-tablet-switch.modules-load.conf /etc/modules-load.d/asus-tablet-switch.conf
sudo modprobe uinput
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=input --action=change
sudo udevadm trigger --subsystem-match=misc --action=change
sudo systemctl daemon-reload
sudo systemctl enable --now asus-tablet-switch.service
sudo systemctl status asus-tablet-switch.service
sudo journalctl -u asus-tablet-switch.service -b
```

If the account already exists, `useradd` will report that fact and can be skipped.
The service has no writable filesystem paths, a private network namespace plus an IP
deny policy, no capabilities, and standard systemd kernel/filesystem/process hardening.
`PrivateDevices` is explicitly disabled because enabling it would hide evdev and uinput.

## Disable or remove

To disable without removing files:

```console
sudo systemctl disable --now asus-tablet-switch.service
```

To remove it completely after disabling:

```console
sudo rm /etc/systemd/system/asus-tablet-switch.service
sudo rm /etc/udev/rules.d/99-asus-tablet-switch.rules
sudo rm /etc/modules-load.d/asus-tablet-switch.conf
sudo rm /usr/local/sbin/asus-tablet-switch
sudo systemctl daemon-reload
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=input --action=change
sudo udevadm trigger --subsystem-match=misc --action=change
sudo userdel asus-tablet-switch
```
