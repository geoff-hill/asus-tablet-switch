# Diagnosing the missing tablet-mode state

This project is based on observations from one ASUS ProArt PX13 HN7306EAC. The
machine was running BIOS HN7306EAC.305, Fedora 44, GNOME 50.3,
`iio-sensor-proxy` 3.8, and a 7.1-series kernel when it was investigated in July
2026. Other firmware revisions and ASUS models may behave differently.

## What the failure looks like

GNOME and libinput can disable the built-in keyboard and touchpad when Linux
reports tablet mode. If that state is wrong or its return transition is missed,
the internal devices can remain disabled after the machine is unfolded. An
external USB or Bluetooth keyboard should normally continue to work.

On the tested machine, Linux exposed an accelerometer and an input device named
`Asus WMI hotkeys`, but it did not expose a dependable physical
`SW_TABLET_MODE` switch. The ASUS device emitted a short burst of `KEY_PROG2`
events at a hinge crossing. Critically, the burst was the same in both
directions. It was therefore a relative toggle, not an absolute report of the
current hinge state.

`monitor-sensor` reported accelerometer orientation and tilt. Those readings
can look like a hinge angle during a simple test, but they describe the motion
of one half of the computer and cannot distinguish all laptop and tablet poses.

## Collect the evidence

Stop this daemon first so its virtual switch does not confuse observations:

```console
sudo systemctl stop asus-tablet-switch.service
```

List the input devices and their advertised capabilities:

```console
cat /proc/bus/input/devices
sudo libinput list-devices
```

Run `evtest`, select `Asus WMI hotkeys`, then slowly fold and unfold the screen:

```console
sudo evtest
```

Look specifically for `KEY_PROG2` and `SW_TABLET_MODE`. Repeat the test several
times, including across suspend and resume. Event numbers are assigned
dynamically, so do not assume that `/dev/input/eventN` remains stable.

Check the lid switch independently by selecting `Lid Switch` in another
`evtest` session. A full close followed by an open should produce an absolute
closed-to-open transition.

For the accelerometer and desktop interpretation, these are useful:

```console
monitor-sensor
sudo libinput debug-events
find /sys/bus/iio/devices -maxdepth 2 -type f -print
journalctl -b -k
journalctl -b -u iio-sensor-proxy.service
```

After reproducing the problem with the daemon enabled, capture its log too:

```console
sudo journalctl -b -u asus-tablet-switch.service
```

In one observed failure, tablet mode was published at 23:48:44, suspend began
at 23:49:23, and resume completed at 23:49:36 without a corresponding laptop
transition. Timestamps like these are more useful in an upstream report than a
description based only on the final stuck state.

## Narrow down the cause

- If restarting this daemon immediately restores the internal keyboard and
  touchpad, its virtual tablet state was probably wrong.
- If an external keyboard works while only the internal devices are disabled,
  libinput's tablet-mode suppression is a likely explanation.
- If raw hinge events disappear from `evtest` while this service is stopped,
  the loss is below this daemon, in firmware or the kernel input driver.
- If external input also stops working, investigate a wider USB, Bluetooth,
  compositor, or kernel problem rather than assuming tablet mode is responsible.
- Check that only one daemon instance exists. Two virtual switches or two
  readers will make state interpretation unpredictable.

Known ways for this toggle-based workaround to lose synchronization include
starting or restarting it while the machine is already folded, losing an event
during device reconnection or resume, firmware changing the number of duplicate
presses, and two genuine crossings occurring inside the debounce interval.

## Why the lid recovery helps

An open lid by itself is ambiguous: both an ordinary laptop and a fully folded
360-degree tablet have an open lid. A newly observed closed-to-open transition
is different. It proves that the display has just left the physically closed
position, so the daemon can safely publish laptop mode and clear its debounce
history. Fully closing and reopening the lid is therefore a deliberate recovery
gesture when the internal input devices remain disabled.

## What a proper upstream fix would provide

The durable fix belongs in firmware or the appropriate ASUS kernel driver. It
would expose an absolute `SW_TABLET_MODE` state, report the current value when
the device is probed, and refresh it after resume. Once the tested hardware does
that reliably, this daemon should be removed rather than kept in the input
stack.

An upstream bug report should include the exact model and BIOS version, kernel
version, `/proc/bus/input/devices`, relevant `evtest` traces in both directions,
suspend/resume journal timestamps, and whether the behavior changes with a
newer kernel or firmware.
