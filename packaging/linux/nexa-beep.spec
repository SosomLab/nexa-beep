# nexa-beep.spec — RPM 명세(rpmbuild · 이미 빌드된 바이너리를 스테이징에서 담는다 · FHS 레이아웃 = build-deb.sh).
#   build-rpm.sh 가 넘기는 매크로: _version · _stagedir(build-deb.sh와 같은 FHS 스테이징 = target/packaging/linux/deb-root/usr)
#   소스 tarball 없이 %install 에서 스테이징을 복사한다 — 빌드는 cargo가 이미 했고(deb와 같은 바이너리), spec은 포장만.
#   debuginfo 추출은 끄고(Cargo profile.release strip=symbols 이미 적용) 바이너리 재strip도 막는다(Rust 실행 파일을 rpm이 다시 만지지 않게).
#   이식 원천: ../nexa-sql/packaging/linux/nexa-sql.spec (10-09).
%global debug_package %{nil}
%global __strip /bin/true
%global __os_install_post %{nil}

Name:           nexa-beep
Version:        %{_version}
Release:        1%{?dist}
Summary:        Zero-config local network messenger (E2E encrypted · sanitized file transfer)
License:        PolyForm-Noncommercial-1.0.0
URL:            https://github.com/SosomLab/nexa-beep
# 아키텍처는 build-rpm.sh의 `rpmbuild --target`(x86_64 · aarch64)이 정한다.
# 런타임 dlopen(Wayland/X11/xkbcommon) — 빌드 시 링크 없음. 자동 의존성 탐지는 glibc·libgcc만 잡는다.
Recommends:     libxkbcommon
Recommends:     libwayland-client
Recommends:     libX11
Recommends:     google-noto-sans-cjk-fonts

%description
Nexa Beep is a zero-configuration local network messenger. There is no server
address to type, no account to create and no contact to register: launch it and
everyone running it on the same LAN appears in the list; pick one and talk.
Messages (text, image, file) are end-to-end encrypted and received files go
through a quarantine/sanitization gate before they materialize. Single static
executable, no runtime dependencies.

Nexa Beep은 제로 컨피그 로컬 네트워크 메신저입니다. 서버 주소 입력·계정 생성·상대 등록이
없습니다. 실행하면 같은 LAN의 사용자가 자동으로 목록에 나타나고, 고르면 즉시 대화합니다.
사용자 데이터는 ~/.config/nexa-beep 에 둡니다.

%prep
# 소스 없음 — 스테이징 복사만.

%build
# cargo가 이미 빌드했다(build-deb.sh).

%install
rm -rf %{buildroot}
mkdir -p %{buildroot}%{_prefix}
cp -a %{_stagedir}/. %{buildroot}%{_prefix}/

%post
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t -f %{_datadir}/icons/hicolor 2>/dev/null || :
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q %{_datadir}/applications 2>/dev/null || :

%postun
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t -f %{_datadir}/icons/hicolor 2>/dev/null || :
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q %{_datadir}/applications 2>/dev/null || :

%files
%{_bindir}/nexa-beep
%{_bindir}/nbeep-imgdec
%{_datadir}/applications/nexa-beep.desktop
%{_datadir}/icons/hicolor/*/apps/nexa-beep.png
%{_datadir}/icons/hicolor/scalable/apps/nexa-beep.svg
%license %{_docdir}/nexa-beep/LICENSE.md
%doc %{_docdir}/nexa-beep/LICENSE.ko.md
%doc %{_docdir}/nexa-beep/README.md
%doc %{_docdir}/nexa-beep/copyright

%changelog
* Fri Oct 09 2026 Sangyong Bae <kiros33@gmail.com> - 0.2.17-1
- 첫 RPM 명세(M5-4c) — deb와 같은 FHS 스테이징을 포장 · nexa-sql 이식.
