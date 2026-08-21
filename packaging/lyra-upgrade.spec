#
# spec file for package lyra-upgrade
#
# Copyright (c) 2026 Rodrigo Brito
# License: GPL-3.0-only
#

Name:           lyra-upgrade
Version:        0.2.0
Release:        0
Summary:        Atualização recuperável do Lyra OS Desktop
License:        GPL-3.0-only
Group:          System/Packages
URL:            https://github.com/britors/Lyra
Source0:        %{name}-%{version}.tar.zst
Source1:        vendor.tar.zst
Source2:        build-source.txt
Source3:        release-signing-key.gpg

BuildRequires:  cargo
BuildRequires:  cargo-packaging
BuildRequires:  desktop-file-utils
BuildRequires:  pkgconfig(gtk+-3.0)
BuildRequires:  pkgconfig(javascriptcoregtk-4.1)
BuildRequires:  pkgconfig(webkit2gtk-4.1)
BuildRequires:  rust >= 1.85
BuildRequires:  zstd
Requires:       curl
Requires:       coreutils
Requires:       dracut
Requires:       gnupg
Requires:       grub2
Requires:       mokutil
Requires:       polkit
Requires:       rpm
Requires:       snapper
Requires:       systemd
Requires:       util-linux
Requires:       zypper
ExclusiveArch:  x86_64

%description
Lyra Upgrade planeja e aplica atualizações do Lyra OS Desktop com preflight,
snapshot Snapper, estado durável, aplicação offline entre releases,
verificação pós-boot e recuperação explícita.

%prep
%autosetup -a1
sed -i 's|^directory = .*|directory = "vendor"|' .cargo/config.toml
test -d vendor
test -s %{SOURCE2}

%build
%{cargo_build}

%install
install -Dm0755 target/release/lyra-upgrade \
    %{buildroot}%{_bindir}/lyra-upgrade
install -Dm0755 target/release/lyra-upgrade-service \
    %{buildroot}%{_libexecdir}/lyra-upgrade-service
install -Dm0755 target/release/lyra-upgrade-offline \
    %{buildroot}%{_libexecdir}/lyra-upgrade-offline
install -Dm0755 target/release/lyra-upgrade-verify \
    %{buildroot}%{_libexecdir}/lyra-upgrade-verify
install -Dm0755 target/release/lyra-upgrade-ui \
    %{buildroot}%{_bindir}/lyra-upgrade-ui
install -Dm0644 packaging/org.lyraos.LyraUpgrade.desktop \
    %{buildroot}%{_datadir}/applications/org.lyraos.LyraUpgrade.desktop
for size in 32 128 256 512; do
    install -Dm0644 src-tauri/icons/${size}x${size}.png \
        %{buildroot}%{_datadir}/icons/hicolor/${size}x${size}/apps/org.lyraos.LyraUpgrade.png
done
install -Dm0644 packaging/org.lyraos.Upgrade.policy \
    %{buildroot}%{_datadir}/polkit-1/actions/org.lyraos.Upgrade.policy
install -Dm0644 packaging/lyra-upgrade-offline.service \
    %{buildroot}%{_unitdir}/lyra-upgrade-offline.service
install -Dm0644 packaging/lyra-upgrade-verify.service \
    %{buildroot}%{_unitdir}/lyra-upgrade-verify.service
install -Dm0644 packaging/90-lyra-refresh.conf \
    %{buildroot}%{_sysconfdir}/zypp/zypp.conf.d/90-lyra-refresh.conf
install -Dm0644 %{SOURCE3} \
    %{buildroot}%{_datadir}/lyra-upgrade/release-signing-key.gpg
install -Dm0644 %{SOURCE2} \
    %{buildroot}%{_datadir}/lyra-upgrade/build-source.txt
install -d %{buildroot}%{_unitdir}/system-update.target.wants
ln -s %{_unitdir}/lyra-upgrade-offline.service \
    %{buildroot}%{_unitdir}/system-update.target.wants/lyra-upgrade-offline.service
install -d %{buildroot}%{_unitdir}/multi-user.target.wants
ln -s %{_unitdir}/lyra-upgrade-verify.service \
    %{buildroot}%{_unitdir}/multi-user.target.wants/lyra-upgrade-verify.service

%check
cargo test --offline --workspace
desktop-file-validate %{buildroot}%{_datadir}/applications/org.lyraos.LyraUpgrade.desktop

%post
%systemd_post lyra-upgrade-offline.service lyra-upgrade-verify.service

%preun
%systemd_preun lyra-upgrade-offline.service lyra-upgrade-verify.service

%postun
%systemd_postun_with_restart lyra-upgrade-offline.service lyra-upgrade-verify.service

%files
%license LICENSE
%doc README.md
%{_bindir}/lyra-upgrade
%{_bindir}/lyra-upgrade-ui
%{_libexecdir}/lyra-upgrade-service
%{_libexecdir}/lyra-upgrade-offline
%{_libexecdir}/lyra-upgrade-verify
%{_datadir}/polkit-1/actions/org.lyraos.Upgrade.policy
%{_datadir}/applications/org.lyraos.LyraUpgrade.desktop
%{_datadir}/icons/hicolor/32x32/apps/org.lyraos.LyraUpgrade.png
%{_datadir}/icons/hicolor/128x128/apps/org.lyraos.LyraUpgrade.png
%{_datadir}/icons/hicolor/256x256/apps/org.lyraos.LyraUpgrade.png
%{_datadir}/icons/hicolor/512x512/apps/org.lyraos.LyraUpgrade.png
%{_unitdir}/lyra-upgrade-offline.service
%{_unitdir}/lyra-upgrade-verify.service
%dir %{_unitdir}/system-update.target.wants
%{_unitdir}/system-update.target.wants/lyra-upgrade-offline.service
%{_unitdir}/multi-user.target.wants/lyra-upgrade-verify.service
%dir %{_sysconfdir}/zypp
%dir %{_sysconfdir}/zypp/zypp.conf.d
%config(noreplace) %{_sysconfdir}/zypp/zypp.conf.d/90-lyra-refresh.conf
%dir %{_datadir}/lyra-upgrade
%{_datadir}/lyra-upgrade/release-signing-key.gpg
%{_datadir}/lyra-upgrade/build-source.txt

%changelog
