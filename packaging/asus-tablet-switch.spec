Name:           asus-tablet-switch
Version:        0.1.0
Release:        1%{?dist}
Summary:        Translate an ASUS hinge hotkey into Linux tablet-mode state

License:        GPL-3.0-only
URL:            https://github.com/geoff-hill/asus-tablet-switch
Source0:        %{url}/releases/download/v%{version}/%{name}-%{version}.tar.gz

BuildRequires:  cargo >= 1.85
BuildRequires:  gcc
BuildRequires:  rust >= 1.85
BuildRequires:  systemd-rpm-macros
Requires:       kmod
Requires:       systemd
Requires:       systemd-udev

%description
A hardware-specific compatibility daemon for the ASUS ProArt PX13. It converts
the observed ASUS WMI hinge hotkey burst into a virtual SW_TABLET_MODE switch
and uses a real lid-open transition as a laptop-mode recovery signal.

%prep
%autosetup

%build
cargo build --release --frozen

%install
install -Dm0755 target/release/%{name} %{buildroot}%{_bindir}/%{name}
install -Dm0644 packaging/99-asus-tablet-switch.rules \
    %{buildroot}%{_udevrulesdir}/99-asus-tablet-switch.rules
install -Dm0644 packaging/asus-tablet-switch.modules-load.conf \
    %{buildroot}%{_modulesloaddir}/asus-tablet-switch.conf
install -Dm0644 packaging/asus-tablet-switch.sysusers \
    %{buildroot}%{_sysusersdir}/asus-tablet-switch.conf
install -Dm0644 packaging/asus-tablet-switch.service \
    %{buildroot}%{_unitdir}/asus-tablet-switch.service
install -Dm0644 docs/asus-tablet-switch.8 \
    %{buildroot}%{_mandir}/man8/asus-tablet-switch.8
sed -i 's|/usr/local/sbin/asus-tablet-switch|%{_bindir}/asus-tablet-switch|' \
    %{buildroot}%{_unitdir}/asus-tablet-switch.service

%check
cargo test --release --frozen

%post
%systemd_post asus-tablet-switch.service
/usr/sbin/modprobe uinput >/dev/null 2>&1 || :
/usr/bin/udevadm control --reload-rules >/dev/null 2>&1 || :
/usr/bin/udevadm trigger --subsystem-match=input --action=change >/dev/null 2>&1 || :
/usr/bin/udevadm trigger --subsystem-match=misc --action=change >/dev/null 2>&1 || :
if [ "$1" -eq 1 ]; then
    printf '%s\n' \
        '' \
        'asus-tablet-switch has been installed but not deliberately enabled.' \
        'Before enabling it, read the test, shutdown, and recovery instructions:' \
        '' \
        '  /usr/share/doc/asus-tablet-switch/README.md' \
        '  /usr/share/doc/asus-tablet-switch/diagnosis.md' \
        '  man 8 asus-tablet-switch' \
        '' \
        'For a non-persistent test, start and later stop it with:' \
        '' \
        '  sudo systemctl start asus-tablet-switch.service' \
        '  sudo systemctl stop asus-tablet-switch.service' \
        '' \
        'If internal input becomes stuck off, fully close and reopen the lid.' \
        'After testing succeeds, enable it as documented in the README.' \
        ''
fi

%preun
%systemd_preun asus-tablet-switch.service

%postun
%systemd_postun_with_restart asus-tablet-switch.service
/usr/bin/udevadm control --reload-rules >/dev/null 2>&1 || :
/usr/bin/udevadm trigger --subsystem-match=input --action=change >/dev/null 2>&1 || :
/usr/bin/udevadm trigger --subsystem-match=misc --action=change >/dev/null 2>&1 || :

%files
%license LICENSE
%doc README.md docs/diagnosis.md
%{_bindir}/asus-tablet-switch
%{_unitdir}/asus-tablet-switch.service
%{_udevrulesdir}/99-asus-tablet-switch.rules
%{_modulesloaddir}/asus-tablet-switch.conf
%{_sysusersdir}/asus-tablet-switch.conf
%{_mandir}/man8/asus-tablet-switch.8*

%changelog
* Sun Jul 19 2026 Geoff Hill <geoff.hill.au@gmail.com> - 0.1.0-1
- Initial package
