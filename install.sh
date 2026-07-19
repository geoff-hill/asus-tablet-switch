#!/usr/bin/env bash

set -euo pipefail

repo_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$repo_dir"

if [[ ! -x target/release/asus-tablet-switch ]]; then
    echo "Missing target/release/asus-tablet-switch; run 'cargo build --release' first." >&2
    exit 1
fi

sudo -v

if ! id -u asus-tablet-switch >/dev/null 2>&1; then
    sudo useradd --system --user-group --home-dir / --shell /usr/sbin/nologin asus-tablet-switch
fi

sudo install -Dm0755 target/release/asus-tablet-switch /usr/local/sbin/asus-tablet-switch
sudo install -Dm0644 packaging/asus-tablet-switch.service /etc/systemd/system/asus-tablet-switch.service
sudo install -Dm0644 packaging/99-asus-tablet-switch.rules /etc/udev/rules.d/99-asus-tablet-switch.rules
sudo install -Dm0644 packaging/asus-tablet-switch.modules-load.conf /etc/modules-load.d/asus-tablet-switch.conf
sudo modprobe uinput
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=input --action=change
sudo udevadm trigger --subsystem-match=misc --action=change
sudo systemctl daemon-reload
sudo systemctl enable asus-tablet-switch.service
sudo systemctl restart asus-tablet-switch.service

sudo systemctl status asus-tablet-switch.service --no-pager || true
sudo journalctl -u asus-tablet-switch.service -b --no-pager
