# ASUS tablet switch daemon

`asus-tablet-switch` is a small, hardware-specific Linux compatibility daemon
for the ASUS ProArt PX13 HN7306EAC. It translates the observed ASUS hinge
hotkey burst into a virtual `SW_TABLET_MODE` switch so GNOME can disable the
internal keyboard and touchpad, and handle tablet behavior, when the display is
folded back.

This is a workaround, not a general ASUS tablet-mode driver. Its useful life
should end when firmware and the kernel expose a reliable absolute tablet-mode
state. Until then, the repository is also intended as a compact example of
investigating an input-stack problem and packaging a narrowly scoped userspace
fix.

Read [Diagnosis and investigation](docs/diagnosis.md) for the evidence behind
the design, commands for separating firmware, kernel, libinput, and daemon
failures, and the information worth collecting for an upstream report.

## Use at your own risk

This program deliberately influences whether the desktop accepts input from the
built-in keyboard and touchpad. A wrong or missed state can leave those devices
disabled until recovery. Test it interactively before enabling the service, keep
an external keyboard available during initial testing, and do not deploy it on
unverified hardware.

The software is provided without warranty, including any warranty of fitness
for a particular purpose. The complete terms and warranty disclaimer are in the
[GNU General Public License version 3](LICENSE), which controls distribution and
use of this project.

## How it works and recovers

The daemon discovers the evdev source by its exact name, `Asus WMI hotkeys`,
rather than relying on an unstable `eventN` number. Each `KEY_PROG2` press toggles
an in-memory laptop/tablet state. A second press less than 750 ms later is
ignored, collapsing the firmware's observed duplicate burst into one transition.
The source is not grabbed, and disconnected devices are rediscovered.

The virtual device is named `ASUS Virtual Tablet Mode Switch`. It starts by
publishing laptop mode (`SW_TABLET_MODE=0`), then alternates between tablet (`1`)
and laptop (`0`).

The daemon also watches the exact evdev device named `Lid Switch`. A real
closed-to-open lid transition proves that the screen has just left the closed
position, so it forces laptop mode and clears the hinge debounce state. If the
toggle becomes inverted and the internal input devices remain disabled, close
the lid completely and reopen it to recover without rebooting.

An already-open lid at startup is not enough to infer a state: both an ordinary
laptop and a fully folded 360-degree tablet have an open lid. Starting or
restarting the daemon while folded, or losing a complete hinge event, can
therefore invert the state until a full lid-close/open recovery or restart. The
750 ms debounce also assumes that two genuine hinge crossings will not occur
inside that interval.

## Build and test

Use stable Rust 1.85 or newer:

```console
cargo build --release
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

Fedora's `evtest` and `libinput-utils` packages provide the runtime inspection
tools used below and in the diagnosis document:

```console
sudo dnf install evtest libinput-utils
```

## Manual test before installation

The source, lid-switch, and uinput nodes require privilege before the service
account and udev rule are installed. Use `sudo` only for this initial test; the
installed daemon runs as a restricted user and does not need network access.

In terminal A:

```console
sudo modprobe uinput
sudo ./target/release/asus-tablet-switch
```

In terminal B, run `sudo evtest` and select `ASUS Virtual Tablet Mode Switch`,
or run `sudo libinput debug-events`. Then:

1. Confirm the initial virtual state is laptop mode, `SW_TABLET_MODE=0`.
2. Fold just past flat. Confirm exactly one `SW_TABLET_MODE=1`; terminal A should
   log one transition and one ignored duplicate `KEY_PROG2` press.
3. Unfold. Confirm exactly one `SW_TABLET_MODE=0`, and that the keyboard and
   touchpad work again.
4. Fold again, close the lid fully, and reopen it. Confirm the daemon logs
   `lid opened: recovered laptop mode` and the internal input devices work.
5. Stop terminal A with Ctrl-C and confirm it logs clean termination.

## Install on Fedora from an RPM 

The RPM build helper makes a vendored source archive from the locked Cargo
dependencies, then builds both a source RPM and a binary RPM without network
access inside `rpmbuild`:

```console
sudo dnf install rpm-build rpmlint systemd-rpm-macros
./packaging/build-rpm.sh
```

The helper uses the `cargo` and `rustc` already on `PATH`, including tools
managed by mise, and passes `--nodeps` to RPM because such tools are intentionally
invisible to RPM's installed-package database. The spec still declares Fedora's
`cargo`, `rust`, and other real build requirements so a normal distribution
builder can resolve them. Skipping the local RPM dependency preflight does not
skip the compile or test phases.

Packages are written below `target/rpmbuild/RPMS` and
`target/rpmbuild/SRPMS`. Install the architecture-specific RPM without enabling
the daemon yet:

```console
sudo dnf install ./target/rpmbuild/RPMS/*/asus-tablet-switch-*.rpm
```

Test the installed service for the current session without enabling it at boot:

```console
sudo systemctl start asus-tablet-switch.service
sudo systemctl status asus-tablet-switch.service
sudo journalctl -b -u asus-tablet-switch.service
```

Repeat the fold, unfold, and full lid-close/open checks from
[Manual test before installation](#manual-test-before-installation), watching
the service journal instead of terminal A. When testing is finished, stop it:

```console
sudo systemctl stop asus-tablet-switch.service
```

If the internal keyboard and touchpad become stuck off, fully close and reopen
the lid. If that does not recover them, use an external keyboard to stop the
service. As a last resort, reboot; because the service has not yet been enabled,
it will not start automatically. Only after the complete test succeeds, enable
and start it persistently:

```console
sudo systemctl enable --now asus-tablet-switch.service
```

The RPM creates the static service account through systemd-sysusers, installs
the udev and modules-load rules, loads `uinput`, and reloads the affected udev
rules. It does not deliberately enable the daemon; Fedora's systemd preset
policy remains authoritative. An upgrade restarts an already running service.

To disable or remove the RPM installation:

```console
sudo systemctl disable --now asus-tablet-switch.service
sudo dnf remove asus-tablet-switch
```

## Manual system installation

For development without an RPM, `install.sh` creates the service account,
installs under `/usr/local` and `/etc`, activates the udev rules, and enables and
restarts the service:

```console
cargo build --release
./install.sh
```

The udev rule grants the daemon group access only to event nodes named exactly
`Asus WMI hotkeys` or `Lid Switch`, plus read/write access to `/dev/uinput`. The
systemd device policy and service hardening provide additional restrictions. In
particular, the process retains no capabilities or root privileges and has no
writable filesystem paths. `PrivateDevices` must remain disabled because it
would hide evdev and uinput.

Disable a manual installation with:

```console
sudo systemctl disable --now asus-tablet-switch.service
```

Its installed files are:

```text
/usr/local/sbin/asus-tablet-switch
/etc/systemd/system/asus-tablet-switch.service
/etc/udev/rules.d/99-asus-tablet-switch.rules
/etc/modules-load.d/asus-tablet-switch.conf
```

## License

Copyright (C) 2026 Geoff Hill. Licensed under the GNU General Public License,
version 3 only. See [LICENSE](LICENSE).
