# 18 · 빌드 & 테스트 (SSOT)

> **이 문서가 빌드·테스트·검증 절차의 SSOT다.** 절차를 바꾼 **그 커밋에서 이 문서를 같이 고친다**(사후 정리 금지).
> 규약 출처 [16 §2-4](16-doc-git-conventions.md#2-문서-작성--필수-규칙-8).

## 1. 명령 (SSOT) — Rust 워크스페이스([07](07-adr-0001-stack.md))

> 로컬·CI 동일. `rust-toolchain.toml`이 stable·컴포넌트(rustfmt/clippy)·4타깃을 자동 고정한다.

| 목적 | 명령 |
|---|---|
| 개발 빌드 | `cargo build --workspace` |
| 릴리스 빌드 | `cargo build --release -p nexa-beep -p nbeep-imgdec` → `target/release/{nexa-beep,nbeep-imgdec}` |
| **테스트**(green 기준) | `cargo test --workspace --features nbeep-core/testkit,nbeep-net/testkit,nbeep-crypto/testkit` — **전부 통과 + 0 ignored 예상 밖 없음** |
| 포맷 | `cargo fmt --all --check` (수정은 `--check` 없이) |
| 린트 | `cargo clippy --workspace --all-targets --features nbeep-core/testkit,nbeep-net/testkit,nbeep-crypto/testkit -- -D warnings` |
| rustdoc 링크 | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --features nbeep-core/testkit,nbeep-net/testkit,nbeep-crypto/testkit` |
| 크로스 빌드 | `cargo build --workspace --target <TARGET>` (예: `aarch64-pc-windows-msvc`) |
| 산출물 크기 | 릴리스 빌드 후 `wc -c target/release/<bin>` — **≤10MB/바이너리**(NFR-B-3) |

**4타깃**: `x86_64-pc-windows-msvc` · `aarch64-pc-windows-msvc` · `x86_64-apple-darwin` · `aarch64-apple-darwin` · `x86_64-unknown-linux-gnu`.
(테스트 실행은 네이티브 3-OS, ARM/타OS는 **빌드 검증만** — [CI](#4-ci--githubworkflowsciyml).)

> **테스트 더블**([13 §11-1](13-code-design-standards.md)): 미완 협력자는 `testkit`(feature 게이트)의 fake/spy로 대체해 단위별로 검증한다. 릴리스엔 미포함.
>
> ★ **테스트 의무 + CI 회귀 게이트(사용자 확정 08-08)** — **모든 목적 기능은 테스트 로직과 함께**만 완료로 친다(테스트 없는 기능 커밋 금지). 완료된 기능의 테스트는 CI `test` 잡(3-OS)이 **매 push·PR마다 전부** 실행한다 — 다른 기능 구현이 기존 동작을 되돌리면 CI가 red가 되어 main 병합이 막힌다(main 항상 green). **새 크레이트에 testkit feature를 추가하면 CI·이 표의 `--features` 목록에 같은 커밋으로 추가**한다 — 빠뜨리면 그 표면이 회귀 게이트 밖에 놓인다.

## 2. 검증 항목 (기능 도착 시 절차화)

| 구분 | 내용 |
| --- | --- |
| 빌드 | 개발 빌드 · 릴리스 빌드 명령, 산출물 경로 |
| 테스트 | 단위 테스트 · 통합 테스트 명령, green 기준 |
| 린트/포맷 | 명령과 CI 강제 여부 |
| **네트워크 검증** | [06 §7](06-network-stack.md) **E-1~E-9** 절차화 — 동일 서브넷 / 무선 혼재 / 클라이언트 격리 AP / 다중 VLAN / 다중 NIC / 링크로컬 직결 / 방화벽 3종 / 100대 / **24h 상주 누수** |
| **종료 경로 검증**(FR-P-7 · R-16) | `SIGINT`/`SIGTERM`(Win 콘솔 종료)으로 **GOODBYE가 실제로 나가는가**(FR-D-8 절반) · 세션·소켓·워커·타이머가 **전부 회수되는가**(NFR-B-6) · 평문 버퍼 zeroize(FR-S-22) · **블로킹 accept/recv에 갇히지 않고 깨어나는가** · 컨테이너는 **`--init` 없이도** 정상 종료되는가 |
| **예산 게이트** | 산출물 크기 · 유휴 RSS · **장시간 상주 후 RSS 증가량 0**(DR-5) — 초과 시 병합 금지 |
| **안전 송수신 검증** | [04](04-safe-transfer.md) 회귀 — 격리물이 실행 불가한가 · 복원 후 MotW/quarantine이 남는가 · Zip Slip/압축폭탄 거부 · RLO 파일명 무해화 |
| **원격 연결 검증** | [19](19-adr-0006-manual-endpoint.md) — 서브넷 밖 인바운드가 **자동 등록되지 않는가** · 대조 전 파일 수신 차단 · 잘못된 핸드셰이크에 **침묵**하는가(포트 스캔 무응답) · 백오프가 유휴 CPU 예산을 깨지 않는가 |
| **저장 암호화 검증** | [17](17-adr-0005-history-at-rest.md) 회귀 — **데이터 폴더 전체를 문자열 스캔해 평문이 한 조각도 없는가**(H-3) · 임시 폴더·썸네일에 평문 없는가 · 키 폐기 후 복호 불가 · 논스 재사용 없음 · 동기화 폴더 감지 동작 |
| **다중 기기 신원 검증** | [20](20-adr-0007-multi-device-identity.md) — **v1**: 대화 스레드·기록·차단 키가 `UserId`인가(`PeerId` 사용 시 컴파일 실패) · 저장 마스터 키가 **래핑 구조**인가(래핑 키 교체 후 재암호화 없이 열리는가) · `sender_device` + 시퀀스로 **중복 제거**되는가. **v2**: 서명 없는 "같은 사용자" 주장 **거부** · 폐기된 기기 키 핸드셰이크 거부 · **옛 목록 재생(version 롤백) 거부** · 목록에 없는 기기의 편입 거부 · **보조 기기에서 서명 시도 실패**(주 기기 전용) · **현재 기기 자기 폐기 불가** · **주 기기 양도 전 폐기 불가** · 복구 시드로 승격 후 **이전 주 기기 폐기 가능** · **상대 기기 구성 변경 시 시스템 라인이 남는가**(끌 수 없음) · `UserId` 키로 **저장 기록이 열리지 않는가**(서명/복호 분리) |
### 2-1. 컨테이너로 실행할 때 — **`--init` 필수**

> DR-19 수동 엔드포인트 실증처럼 **리눅스 노드를 컨테이너로 띄울 때**의 규약.

```bash
docker run --rm --init -p 47200:47200 \
  -v "$PWD/.docker-target/release/nexa-beep:/nexa-beep:ro" \
  debian:stable-slim /nexa-beep --serve 47200
```

**`--init`을 빠뜨리면 Ctrl+C로 종료되지 않는다.** 리눅스 커널은 **PID 1에게 시그널 기본 동작을 적용하지 않아서**, 핸들러가 없는 프로세스가 PID 1이면 `SIGINT`·`SIGTERM`을 **그대로 무시**한다. `--rm`은 컨테이너가 종료돼야 발동하므로 컨테이너도 남는다.

| 조건 | `docker stop` 소요 | 해석 |
|---|---:|---|
| `--init` 없음 | **10.26초** | SIGTERM 무시 → 10초 타임아웃 → **SIGKILL** |
| **`--init`** | **0초** | tini가 시그널 중계 → 자식은 PID 1이 아니므로 정상 종료 |

> 실측 2026-08-08([journal](journal/2026-08-08.md)). ⚠️ **`--init`은 우회이지 해결이 아니다** — 앱 자체의 종료 경로 부재는 **FR-P-7 · R-16**으로 등록돼 있다. 정리가 안 될 때는 `docker stop <id>`(10초) 또는 `docker kill <id>`(즉시).

## 3. 규칙

- **main은 항상 green** — 테스트·린트 통과 전 병합 금지.
- **[13 §12 코드 리뷰 체크리스트](13-code-design-standards.md) 통과**가 병합 조건이다(파이프라인 우회 없음·포트 주입·민감 타입 마스킹·상한 있는 큐 등).
- 네트워크 기능은 단위 테스트만으로 신뢰하지 않는다. **2대 이상 실기 확인**을 병합 조건에 포함하고, 결과 수치를 journal에 남긴다.

## 4. CI — `.github/workflows/ci.yml`

> main push·모든 PR에서 실행. 하나라도 실패하면 **병합 금지**(§3).

| 잡 | 실행 | 게이트 |
|---|---|---|
| **lint** | ubuntu | `fmt --check` · `clippy -D warnings` · `rustdoc -D warnings` |
| **test** | ubuntu·macOS·windows(네이티브) | `build` + `test`(testkit). 3-OS = 중립 크레이트 4타깃 검증의 실행부 |
| **cross-build** | Win ARM64 · Intel mac | 실행 불가 타깃은 **컴파일 검증만** |
| **budget** | ubuntu | 릴리스 빌드 후 **바이너리 ≤10MB**(NFR-B-3) 초과 시 실패 |

- 전역 `RUSTFLAGS=-D warnings` · `RUSTDOCFLAGS=-D warnings` — 경고 = 실패.
- ⏳ **미발효(실물 도착 후 추가)**: 임포트 화이트리스트(NFR-B-5 · OS 인박스만 — SP-1/M0-2) · 유휴 RSS·24h 누수(NFR-B-1/6 — M1+) · 안전 회귀([§2](#2-검증-항목-기능-도착-시-절차화)) · 발견 실측 E-1~E-9(실기).
