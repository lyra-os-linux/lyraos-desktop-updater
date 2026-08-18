#
# spec file for package lyra-upgrade
#
# Copyright (c) 2026 Rodrigo Brito
# License: GPL-3.0-only
#

Name:           lyra-upgrade
Version:        0.1.0
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
Requires:       dracut
Requires:       gnupg
Requires:       grub2
Requires:       polkit
Requires:       snapper
Requires:       systemd
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
install -Dm0644 packaging/org.lyraos.Upgrade.policy \
    %{buildroot}%{_datadir}/polkit-1/actions/org.lyraos.Upgrade.policy
install -Dm0644 packaging/lyra-upgrade-offline.service \
    %{buildroot}%{_unitdir}/lyra-upgrade-offline.service
install -Dm0644 packaging/lyra-upgrade-verify.service \
    %{buildroot}%{_unitdir}/lyra-upgrade-verify.service
install -Dm0644 %{SOURCE3} \
    %{buildroot}%{_datadir}/lyra-upgrade/release-signing-key.gpg
install -Dm0644 %{SOURCE2} \
    %{buildroot}%{_datadir}/lyra-upgrade/build-source.txt
install -d %{buildroot}%{_unitdir}/system-update.target.wants
ln -s %{_unitdir}/lyra-upgrade-offline.service \
    %{buildroot}%{_unitdir}/system-update.target.wants/lyra-upgrade-offline.service

%check
cargo test --offline --workspace
desktop-file-validate %{buildroot}%{_datadir}/applications/org.lyraos.LyraUpgrade.desktop

%pre
getent group lyra-upgrade >/dev/null || groupadd -r lyra-upgrade

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
%{_unitdir}/lyra-upgrade-offline.service
%{_unitdir}/lyra-upgrade-verify.service
%{_unitdir}/system-update.target.wants/lyra-upgrade-offline.service
%dir %{_datadir}/lyra-upgrade
%{_datadir}/lyra-upgrade/release-signing-key.gpg
%{_datadir}/lyra-upgrade/build-source.txt

%changelog
