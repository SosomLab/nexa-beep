# 48 · ADR-0015 결정 사항 상세 — 추가 조사 · 설계 변경 · 기능/성능/보안 영향 · 흐름도

> **작성 2026-09-06** · 대상 = [46 ADR-0015](46-adr-0015-userid-handle-passphrase.md) + [47 검토 보고서](47-adr-0015-review-userid-continuity.md) · 브랜치 `feat/userid-handle`
> **사용자 요청**: *"추가 결정이 필요한 사항을 상세히 조사(혹은 물어보고) · 결정대로 개발했을 때 설계 변경 상세 · 기능·성능 영향 · 보안 영향 · Mermaid로 흐름·순서·개념."*
> **읽는 법**: §0 표 → §2 개념도 → §3 흐름도(시퀀스 8) → §4 결정별 상세 → §5 성능 · §6 보안 종합. 전부 **47의 3층 구조** 를 전제로 쓴다. 🔴 표시 항목은 **09-06 사용자 답변으로 전부 권고안 확정**(§8 · 46 §9).

---

## 0. 결정 9문항 — 권고 · 근거 · 영향 등급

| # | 질문 | 권고 | 한 줄 근거 | 기능 | 성능 | 보안 |
|:--:|---|---|---|:--:|:--:|:--:|
| D-32-1 | 형제 기기 소속 증명 | **XXpsk3 + UserKey 서명 목록** | PSK = 형제 사이 증명, 서명 = 제3자 검증. 둘이 다른 일을 한다(§4-1) | 대 | 소 | **대** |
| D-32-2 | (47로 대체) | — | — | | | |
| 🔴 D-32-3 | 제약 해제 범위 | **사람 판정 전부 해제 · 파일 판정 유지** | DR-13은 발신자 무관(§4-3) | 대 | — | 중 |
| 🔴 D-32-4 | 동기화 1차 범위 | **sender copy + 따라잡기 200줄** | copy만이면 "꺼졌던 기기"가 비어 남는다(§4-4) | 대 | 중 | 소 |
| 🔴 D-32-5 | 컨텐츠 모드 착수 | **S0~S4 실기 후 별도 결정** | 서버 저장·쿼터·배포가 동반(§4-5) | 대 | 대 | 대 |
| 🔴 D-32-6 | 패스프레이즈 저장 | **`profile.sec` 봉인** | 평문이면 폴더 복사 = 전 기기 탈취(§4-6) | — | — | 중 |
| 🔴 D-32-7 | 강도 정책 | **표시+경고 · 자동 추천 12자** | 온라인 추측만 가능하므로 강제는 과하다(§4-7) | 소 | — | 중 |
| D-32-8 | UserId 원천 | **무작위 UserKey(전 기기 복제 · 봉인)** | 암호 변경 ≠ 신원 변경(47) | 대 | 소 | **대** |
| D-32-9 | 폐기 경쟁 | **정직한 충돌 표시** | 자동 승자 = 공격자 승리(47 §3-3) | 중 | — | **대** |

---

## 1. 추가 조사 결과 (사실 · 09-06)

| # | 조사 | 결과 | 설계 반영 |
|:--:|---|---|---|
| F-1 | `snow 0.9.6` PSK API | `Builder::psk(location, key)` (`builder.rs:105`) · `Token::Psk(n)` 처리 `handshakestate.rs:259` · **psk 패턴이면 `e` 토큰 뒤 `MixKey(e.pub)`** (`:243, :369` `is_psk()`) | 의존 신규 0 · `initiate_psk/accept_psk` 두 함수로 끝 |
| F-2 | **인바운드에서 XX / XXpsk3를 어떻게 가르나** | F-1의 MixKey(e) 때문에 **XXpsk3의 1번 메시지 payload는 e.pub에서 파생된 키로 AEAD 봉인**된다(비밀 없이 누구나 열 수 있지만 **인증 태그**가 붙는다). 응답자가 XXpsk3 상태로 1번을 파싱해 태그가 맞고 payload가 마커 `NBPSK1`이면 psk 경로, 태그 실패면 XX로 재파싱. **DH 없이 해시 1회 + AEAD 1회** — 추가 왕복 0 | §3-2 시퀀스 · **실측 항목 T-1** |
| F-3 | 서명 의존 | `curve25519-dalek 4.1.3`이 이미 트리에 있다(snow). **`ed25519-dalek 2.x`는 같은 4.x를 쓴다** → 트리 증가 = 크레이트 1개. 3.0은 `curve25519-dalek 5`라 **중복 트리** — 2.x로 핀 | DR-5 산출물 +약 50~80KB(추정 · 실측 T-2) |
| F-4 | 발견 패킷 꼬리 확장 | `wire.rs:138` `bytes.len() != FIXED + name_len` → **길이 엄격** · 꼬리 확장 불가 · 미지 `WIRE_VER`는 `None`으로 조용히 폐기 | LAN 힌트는 **별도 패킷**(`kind = UserHint` · 새 ver) — 구버전은 폐기, 신버전만 읽음. 힌트는 **최적화**이고 없어도 §3-4 폴백으로 동작 |
| F-5 | 힌트 없이 형제를 만났을 때 | XX 세션 → `UserHello`에서 같은 UserId **주장** → 증명이 없으므로 **XXpsk3로 재세션**(1회 추가 핸드셰이크) → 성립 시 형제 확정 | 릴레이 첫 만남(RID_pair)은 힌트 자체 · LAN은 힌트 패킷 또는 폴백 |
| F-6 | 폐기 경쟁 완화 시간 | 후계 증명서는 **세션 성립 시 즉시** 제시(UserHello에 동봉) · 상대가 오프라인이면 다음 접촉 | 시간 우위는 "먼저 접촉한 쪽" — 정당 사용자는 기기 대부분과 이미 세션이 있어 유리 |

---

## 2. 개념도

### 2-1. 3층 신원과 키 파생

```mermaid
flowchart TB
  subgraph L3["③ 문 열쇠 — 사용자가 입력 · 바꿔도 신원 불변"]
    H["handle (라벨)"]
    P["passphrase (비밀)"]
    KP["KP = PBKDF2-HMAC-SHA256(passphrase, 'nbeep-user-kdf-v1'‖handle, 60k)"]
    H --> KP
    P --> KP
    PSK["PSK = H('psk-v1'‖KP)\n형제 세션 XXpsk3"]
    RID["RID_pair(day) = H('rid-v1'‖KP‖day)[..16]\n릴레이 만남 3개"]
    TAG["LAN_tag(day) = H('lan-v1'‖RID_pair)[..16]\n발견 힌트"]
    KW["K_wrap_user = H('wrap-v1'‖KP)\n봉인 열쇠"]
    KP --> PSK & RID & TAG & KW
  end
  subgraph L2["② 신원 — 무작위 · 불변 · 남이 나를 가리키는 값"]
    UK["UserKey (Ed25519 쌍)\n최초 기기 1회 생성 · 형제에 복제"]
    UID["UserId = H('id-v2'‖UserKey.pub)"]
    UK --> UID
    SIG["서명 대상: UserHello(기기 목록·version)\nSuccession · 서버 챌린지"]
    UK --> SIG
    KM["K_user_master (컨텐츠 모드)"]
  end
  subgraph L1["① 앵커 — 기기 정적 키 · 불변"]
    PID["PeerId (X25519 pub)\n핀 · 세션 · 후계 판정 근거"]
  end
  KW -. "봉인/해제" .-> UK
  KW -. "봉인/해제" .-> KM
  PID -- "Noise XX / XXpsk3" --> SES["세션"]
  UID -- "스레드·차단·프로필 키" --> TH["대화 스레드 (V1-1 실반영)"]
  PID -- "소속 주장 (서명된 목록)" --> UID
```

### 2-2. 신뢰의 출처 — 누가 무엇을 증명하나

```mermaid
flowchart LR
  A["형제 기기 A"] -- "XXpsk3 성립 = KP를 안다" --> B["형제 기기 B"]
  A -- "UserHello{devices, ver, sig(UserKey)}" --> C["다른 사용자 C"]
  C -- "검증: sig ∈ UserId · 제시한 PeerId ∈ devices · ver 단조" --> C2["C의 판단: A는 UserId_X 소속"]
  A -. "핸들 'kiros33' (라벨 · 서명 대상 아님)" .-> C
  classDef proof fill:#dfe,stroke:#2a7;
  class B,C2 proof
```

---

## 3. 흐름도 (시퀀스)

### 3-1. 부트스트랩 — 첫 기기와 두 번째 기기

```mermaid
sequenceDiagram
  autonumber
  participant U as 사용자
  participant A as 기기 A (첫 기기)
  participant R as nexa-beepd (릴레이 · 무변경)
  participant B as 기기 B (두 번째)
  U->>A: 설정 › 사용자: handle + passphrase 입력
  A->>A: KP 계산(PBKDF2 60k · 메모리) · UserKey 생성 · K_wrap_user로 봉인 저장(user.key)
  A->>R: Register{ rids_around(PeerId_A) 3 + RID_pair 3 }
  U->>B: 같은 handle + passphrase 입력
  B->>B: KP 계산 · UserKey 없음 → "형제를 기다림"
  B->>R: Register{ rids_around(PeerId_B) 3 + RID_pair 3 }  (RID_pair는 최신 등록자 = B가 점유)
  A->>R: Open(RID_pair)  (아무 형제와도 미연결일 때만 · A가 다이얼 차례라면)
  R-->>B: Incoming{ch}
  A->>B: Noise XXpsk3 (PSK = H(KP))  — 홀펀칭 → 릴레이 폴백 사다리 그대로
  Note over A,B: 성립 = 같은 KP · 서로의 PeerId 확정 · 이후엔 기기별 RID로 직접 연다
  A->>B: UserHello{ user_pub, name, devices:[A], ver:1, sig } + UserKeyBlob(봉인본)
  B->>B: K_wrap_user로 해제 → UserKey 보유 · UserId 동일 확인
  B->>A: UserHello{ …, devices:[A,B], ver:2, sig }  (B가 서명 · 이후 목록 공유)
  A-->>B: SyncPull(스레드별 after_seq) / B-->>A: SyncLines  (따라잡기 · §3-6)
```

### 3-2. 인바운드 패턴 판별 — XX인지 XXpsk3인지 (추가 왕복 0)

```mermaid
sequenceDiagram
  autonumber
  participant I as 개시자
  participant Rsp as 응답자
  I->>Rsp: msg1 = e ‖ payload  (XXpsk3면 payload = AEAD_{k(e.pub)}("NBPSK1") · XX면 평문 빈 payload)
  Rsp->>Rsp: ① XXpsk3 상태로 msg1 파싱 시도 — MixKey(e.pub) → AEAD 열기
  alt 태그 OK ∧ payload == NBPSK1
    Rsp->>Rsp: psk 경로 확정 · Builder.psk(3, PSK_mine)
    Rsp->>I: msg2 (XXpsk3)
    I->>Rsp: msg3 (psk 혼합 · PSK 다르면 여기서 복호 실패)
    alt 성립
      Note over I,Rsp: 형제 확정 → UserHello 교환
    else 실패
      Rsp->>Rsp: 실패 카운터(IP·PeerId) · 5회/분 초과 = 10분 무시
      I->>Rsp: (선택) XX로 재시도 — "같은 사용자가 아니다"는 정상 결과
    end
  else 태그 실패
    Rsp->>Rsp: ② XX 상태로 재파싱(현행 경로)
    Rsp->>I: msg2 (XX)
  end
```

> ★ 실측 항목 T-1: snow에서 msg1 파싱 실패 후 **새 HandshakeState로 같은 바이트를 재파싱**하는 것은 상태가 독립이라 문제없다(각 Builder가 새 상태). 비용 = 해시 2회 + AEAD 1회.

### 3-3. 다른 사용자와의 세션 — 서명된 기기 목록으로 스레드 접기

```mermaid
sequenceDiagram
  autonumber
  participant A1 as 나 · 기기 A1 (UserId_X)
  participant C as 상대 C (다른 사용자)
  A1->>C: Noise XX (현행 그대로 · TOFU 핀은 PeerId_A1)
  A1->>C: Control tag 4 UserHello{ user_pub_X, name, devices:[A1,A2], ver:7, sig_X }
  C->>C: ① sig 검증(user_pub_X) ② PeerId_A1 ∈ devices ③ ver ≥ seen_max(X)
  alt 검증 통과
    C->>C: trust 레코드(A1).user = X · seen_max(X)=7 · 스레드 키 = X (A1·A2 라인 병합)
    C->>C: 목록: "kiros33 (기기 2)" 한 행
  else 실패(서명 불일치·목록 밖·version 역행)
    C->>C: 세션 유지 · 소속 미확정 · 행은 PeerId 단독 · 상태바 1줄
  end
  Note over C: A2가 나중에 붙으면 A2도 같은 절차 → 같은 X로 접힌다. C는 A1이 준 목록만으로 A2를 믿지 않는다(A2 자신의 세션+서명이 근거).
```

### 3-4. 힌트 없이 만난 형제 — 폴백(XX → 주장 → XXpsk3 재세션)

```mermaid
sequenceDiagram
  autonumber
  participant A as 기기 A
  participant B as 기기 B (같은 사용자 · 힌트 없음)
  A->>B: Noise XX (일반 상대로 간주)
  A->>B: UserHello{ user_pub_X … }
  B->>B: 내 UserId도 X — "형제 후보" (아직 증명 없음)
  B->>A: UserHello{ user_pub_X … }
  A->>B: 새 세션 XXpsk3 (기존 XX 세션은 성립 즉시 닫는다 · 사전순 작은 쪽이 개시)
  Note over A,B: 성립 = 형제 확정. 실패 = "같은 UserId를 주장하지만 KP가 다르다" → 사칭 경고(이름 충돌보다 강한 등급)
```

### 3-5. 메시지 발신 — sender copy와 확인 응답

```mermaid
sequenceDiagram
  autonumber
  participant A1 as 나 · A1 (발신)
  participant A2 as 나 · A2 (형제)
  participant C1 as 상대 · C1
  participant C2 as 상대 · C2
  A1->>A1: ChatMessage{ sender_device: A1, seq: n, body }
  par fanout(recipients = 상대 기기 ∪ 내 형제)
    A1->>C1: msg
    A1->>C2: msg
    A1->>A2: msg (sender copy)
  end
  C1-->>A1: Delivered/Read ack (검증 상대라면)
  C2-->>A1: ack
  A2->>A2: sender_device ∈ 내 UserId → 오른쪽 말풍선 · 알림 없음 · ack 안 보냄
  A1->>A2: Control tag 5 SyncRead{ thread, upto_seq }  (읽음 상태 동기 · 상대에게는 안 나감 — DR-25)
  Note over C1,C2: 상대가 회신하면 C1은 내 기기 A1·A2 전부에 팬아웃(내 목록은 UserHello로 안다). 꺼진 A2는 못 받는다 → §3-6
```

### 3-6. 따라잡기 — 꺼졌던 기기가 돌아왔을 때

```mermaid
sequenceDiagram
  autonumber
  participant A2 as A2 (방금 켜짐)
  participant A1 as A1 (계속 켜져 있던 형제)
  A2->>A1: XXpsk3 성립 · UserHello
  A1->>A2: UserHello{ ver } + ThreadDigest[ (thread, sender_device, max_seq) … ]
  A2->>A2: 내 digest와 비교 → 부족 목록
  loop 스레드별 (최근 200줄 · 세션당 1MiB 상한)
    A2->>A1: Control tag 6 SyncPull{ thread, after:(device,seq), max }
    A1->>A2: Control tag 7 SyncLines[ ChatLine… ] (세션 암호화 · 평문 라인)
    A2->>A2: dedup (sender_device, seq) → 내 키로 봉인 저장(history/u-X.seg) · 읽음 상태는 SyncRead 값 적용
  end
  Note over A2: 파일 본문은 오지 않는다 — "[파일] … (이 기기에는 없음)" 라인만. 파일은 컨텐츠 모드(§3-8)
```

### 3-7. 암호 변경 vs 기기 분실 — 무엇이 바뀌고 무엇이 안 바뀌나

```mermaid
sequenceDiagram
  autonumber
  participant U as 사용자
  participant A as A (남은 기기)
  participant L as L (잃은 기기)
  participant C as 상대 C
  rect rgb(235,245,235)
  Note over U,C: (가) 암호만 변경 — 신원 불변
  U->>A: 새 passphrase
  A->>A: KP' 계산 → UserKey·K_user_master 재래핑(재암호화 없음) · PSK'/RID' 갱신
  Note over A,C: C는 아무것도 모른다. 형제들은 새 암호 입력 시 PSK'로 재결합. L은 옛 PSK라 형제 세션 불가
  end
  rect rgb(250,235,235)
  Note over U,C: (나) 기기 분실 — 침해 대응 = UserKey 교체
  U->>A: "기기 L 분실 · 침해 대응"
  A->>A: UserKey' 생성 · Succession{ old_pub, new_pub, devices:[A…], revoked:[L], ver:n+1, sig_old, sig_new }
  A->>C: UserHello(new) + Succession (다음 세션)
  C->>C: sig_old ∈ 핀한 X ∧ 제시자 A ∈ 핀된 PeerId ∧ ver>seen → 스레드·핀을 X'로 접음 · L = revoked
  L->>C: (공격자) Succession'{ new_pub_L, revoked:[A], ver:n+1, sig_old }  — 옛 키를 쥐고 있으므로 만들 수 있다
  C->>C: 같은 ver · 다른 후계 → ★ 충돌: 둘 다 미확정 · 스레드는 X에 머묾 · 화면에 "P_A와 P_L이 다른 후계를 주장" (D-32-9)
  U-->>C: 다른 채널로 확인 → C가 하나를 선택
  end
```

### 3-8. 컨텐츠 모드 — 동기 저장소와 파일 (S5 · 별도 결정)

```mermaid
flowchart LR
  subgraph Dev["내 기기들 (UserId_X)"]
    A1["A1"]; A2["A2"]
  end
  subgraph Srv["nexa-beepd · Content 타입 (봉투만)"]
    IDX["스레드 색인 Enc(K_user_master)"]
    LB["라인 블록 Enc(K_thread, lines[seq..])"]
    BL["파일 blob Enc(K_content, file)\n+ 봉투① Wrap(K_user_master, K_content)\n+ 봉투② Wrap(상대, K_content)"]
    CUR["기기별 커서 Enc(K_user_master)"]
  end
  C["상대 C (UserId_Y)"]
  A1 -- "PutBlob(서명 챌린지 인증)" --> LB & BL
  A2 -- "GetIndex/GetBlob(cursor 이후)" --> IDX & LB & BL & CUR
  A1 -- "LAN 직접(S-1 우선) 또는 서버 경유" --> C
  C -- "봉투② 열기 → GetBlob → 무해화 게이트 → 격리" --> BL
  A2 -- "온라인 보관함 클릭 = pull → 격리 → 승인 실체화(기기별)" --> BL
```

```mermaid
sequenceDiagram
  autonumber
  participant A1 as A1
  participant S as Content 서버
  participant A2 as A2 (나중에 켜짐)
  participant C as 상대 C
  A1->>S: Auth: 챌린지 ← S · 응답 = Sign(UserKey, nonce) → PseudoId = HMAC(salt_s, UserId)
  A1->>S: PutBlob(file: Enc(K_content)) + 봉투①(K_user_master) + 봉투②(C)
  A1->>C: 파일 오퍼(기존 와이어) — blob_id + 봉투② (본문은 서버)
  C->>S: GetBlob(blob_id)  → 무해화 → 격리 → 승인
  A2->>S: GetCursor · GetIndex · GetBlob(부족분)  — 켜졌을 때
  A2->>A2: 봉투① 해제 → 파일 미리보기/격리 · 라인 블록 → 스레드 병합
  Note over S: 서버가 아는 것 = 가명 · blob 크기 · 시각 · 봉투 개수. 이름·내용·상대의 정체 = 모른다(S-3)
```

### 3-9. 기기 소속 상태도

```mermaid
stateDiagram-v2
  [*] --> Unpaired: 핸들·암호 미입력
  Unpaired --> Seeking: 핸들·암호 입력 (KP · RID_pair 등록 · 힌트 송출)
  Seeking --> Bootstrapped: 형제 없음 → UserKey 생성 (첫 기기)
  Seeking --> Member: XXpsk3 성립 + UserKey 수신
  Bootstrapped --> Member: 형제와 첫 XXpsk3 (동시 부트스트랩이면 created_at 빠른 키로 병합 · 진 쪽 Succession)
  Member --> Member: 암호 변경 (재래핑 · UserId 불변)
  Member --> Rotated: 침해 대응 (UserKey' · Succession)
  Rotated --> Member: 형제들이 새 목록 수용
  Member --> Revoked: 다른 기기가 나를 revoked에 넣음 (다음 접촉 시 상대가 반영)
  Member --> Unpaired: 사용자가 "신원에서 분리" (UserKey 폐기 · PeerId 단독 노드로)
```

---

## 4. 결정별 상세 — 설계 변경 · 기능 · 성능 · 보안

### 4-1. D-32-1 소속 증명 = XXpsk3 + 서명 목록 (권고 확정)

| 항목 | 내용 |
|---|---|
| **설계 변경** | `nbeep-crypto::noise`: `PARAMS_PSK = "Noise_XXpsk3_25519_ChaChaPoly_BLAKE2s"` · `initiate_psk(link, id, psk)`/`accept_any(link, id, psk_opt)`(§3-2 판별) · msg1 payload 마커. `nbeep-core`: `UserHello`(Control 4) 인코더/서명 검증 · `TrustRecord.user/list_ver/seen_max`. 문서: 46 §3-2·§5-3 개정, [08 ADR-0002](08-adr-0002-discovery-transport.md) §4에 "psk 변종" 절 추가 |
| **기능** | 형제 기기 = 승인 0회(요구 R-2 근거) · 제3자는 서명 목록으로 기기 N대를 한 행으로 접는다 · 힌트 없어도 폴백(§3-4)으로 성립 |
| **성능** | XXpsk3는 XX와 **DH 횟수 동일**(psk는 해시 혼합) → 핸드셰이크 비용 ≈ 동일 · 판별 = 해시 2회+AEAD 1회 · 폴백 경로는 핸드셰이크 1회 추가(형제 첫 만남 1회뿐) · Ed25519 검증 ≈ 0.1ms/UserHello(세션당 1회) |
| **보안 +** | 사람의 SAS 대조가 빠진 자리를 **암호학적 증명**이 채운다 · 도청자는 핸드셰이크 기록으로 암호를 검증 못 한다(온라인 추측만) · A-1(남의 기기 편입 주장)은 **서명**으로 막힌다(46 원안의 "재확인"보다 강함) |
| **보안 −/잔여** | **PSK 실패 오라클**: 응답자가 "psk 불일치"로 끊는 사실 자체가 추측 1회의 답 → 실패 백오프 필수(§3-2) · 형제 세션은 **전방 비밀성은 유지**(임시 DH)하나 PSK 유출 = 이후 모든 형제 세션 성립 가능(=암호 유출과 동일) · 서명 키 유출(=UserKey) = 제3자에게 기기 편입 가능 → 47 §3-3 |

### 4-2. D-32-8 UserId = H(무작위 UserKey.pub) · 전 기기 복제 (권고 확정)

| 항목 | 내용 |
|---|---|
| **설계 변경** | `nbeep-crypto::userkey`: Ed25519 생성·서명·검증(`ed25519-dalek 2`) · `user.key` 봉인 파일(sealed · 도메인 `user-key-v1` · K_wrap_user) · 형제 세션 성립 시 `UserKeyBlob` 동기(Control 9) · 동시 부트스트랩 병합 규칙(created_at → pub 사전순) · 46 §3-1 파생식에서 `UserId` 줄 교체 |
| **기능** | 암호·핸들 변경이 신원을 안 바꾼다 · 어느 기기에서든 기기 추가 · 복구 시드 불필요(암호+살아 있는 기기 1대) |
| **성능** | 키 생성 1회 · 파일 +100B · 세션당 서명 1회/검증 1회 · 무시 가능 |
| **보안 +** | 선점 불가(256비트 무작위) · 서버 이름공간은 서명 챌린지 · 후계 증명서로 연속성(47 §3-2) |
| **보안 −/잔여** | **UserKey가 전 기기에** → 기기 1대 침해 = 서명 능력 탈취 → 폐기 경쟁(47 §3-3) · 완화 = 정직한 충돌(D-32-9) + 암호 동시 변경으로 형제 세션 차단 · **v2 주 기기 승격** 경로 유지 |

### 4-3. 🔴 D-32-3 제약 해제 범위

| 해제 후보 | 권고 | 기능 영향 | 보안 영향 |
|---|:--:|---|---|
| TOFU 첫 접촉 절차 | 해제 | 형제는 즉시 `Verified` 표시 | 근거 = PSK(4-1) · 위험 없음 |
| SAS/`/verify` | 해제 | 권유 줄 없음 | 동일 |
| 파일 **승인**(미왕래 강등 포함) | 해제 | 형제가 보낸 파일 = 자동 승인 → 격리 | **격리는 유지**되므로 실행 위험 없음. 잔여 = 형제 기기가 침해됐을 때 악성 파일이 격리함까지는 자동으로 들어온다(실체화는 여전히 사람) |
| 원격 인바운드 요청 대기(FR-S-25) | 해제 | 다른 망의 내 기기가 모달 없이 붙는다 | 근거 = PSK · 위험 없음 |
| 원격 파일 차단(대조 전) | 해제 | 다른 망의 내 기기와 파일 | 동일 |
| OS 알림 무음 | 해제(단 sender copy는 무알림) | 형제 메시지 알림 정상 | — |
| **격리·무해화·기기별 실체화** | **유지** | "일반 메신저"와의 마지막 차이 | DR-13 · 발신자 무관 |
| **차단 단위** | PeerId → UserId | 상대 사용자 차단 = 기기 전부 | ADR-0007 §9-2 |

> 물음: 파일 승인까지 자동으로 할지(권고) vs 남길지. 남기면 "형제가 보낸 파일도 승인 창"이 뜬다 — 요구 R-2와 충돌하지만 침해 기기 방어는 한 겹 더 남는다.

### 4-4. 🔴 D-32-4 동기화 1차 범위

| 안 | 기능 | 성능 | 보안 |
|---|---|---|---|
| ⓐ sender copy만 | 켜져 있는 기기끼리만 같은 대화 · 꺼졌던 기기는 그 기간이 **비어 남는다** | 발신 대역 × (형제 수) | 형제 세션 안 · 추가 면 없음 |
| **ⓑ + 따라잡기 200줄** | 꺼졌던 기기도 복귀 시 최근 대화 복원 · 읽음 상태 동기 | 복귀 시 스레드당 ≤200줄 · 세션당 ≤1MiB(설정) · 대량 이력은 컨텐츠 모드 | 형제 세션 안 · **침해 형제가 라인 주입 가능**(dedup 키가 같으면 무시 · 새 seq면 들어온다) → 형제는 이미 전면 신뢰이므로 새 면이 아님 |
| ⓒ + 전체 이력 | 전 기록 복원 | 대량 · 예산 위험 | 동일 |

### 4-5. 🔴 D-32-5 컨텐츠 모드 착수 시점

| | 지금(S4 직후) | **실기 후 별도 결정** |
|---|---|---|
| 필요한 것 | beepd `Content` 와이어 5종 · 저장·쿼터·TTL · 서명 인증 · 배포(`beepd-v*`) · 실 NAT 실기 · 보관함 온라인(X-3) | S0~S4를 2-PC로 실기해 **따라잡기만으로 충분한지** 먼저 본다 |
| 성능 | 서버 디스크·대역(쿼터 설계 없이는 무제한) | — |
| 보안 | 서버가 **사용자별 암호문 덩어리를 장기 보관**(L1 90일 기본) · 가명 이름공간 · 쿼터 소진 공격 · 서명 챌린지 재생 방지 | — |
| 권고 이유 | "일반 메신저처럼"의 마지막 조각이지만 **범위가 다른 마일스톤**이고, 사용자 마스터 키(암호 = 전 기록의 열쇠)라는 **새 위협 모델**을 함께 열어야 한다 | |

### 4-6. 🔴 D-32-6 패스프레이즈 저장

| 안 | 내용 | 보안 |
|---|---|---|
| **ⓐ `profile.sec` 봉인**(권고) | 기존 PII 사이드카(기기 키 파생 봉인 · 0600) | 폴더 복사만으로는 못 읽는다(identity.key 동반 필요 — 그건 이미 신원 탈취) |
| ⓑ `settings.cfg` 평문(clip 동일) | 설정 백업/복원에 그대로 포함 | 백업 파일·동기화 폴더에 평문 노출 |
| ⓒ 저장 안 함(매 기동 입력) | — | 제로컨피그·자동 실행과 충돌 |

### 4-7. 🔴 D-32-7 강도 정책

온라인 추측만 가능(§4-1) + 실패 백오프 → 초당 시도가 수 회로 제한된다. 12자 자동 추천(≈59비트)이면 실질 안전. 사용자가 짧은 암호를 고르면 **경고만**(clip D-29 동일). 강제 최소 길이는 "내 기기끼리 묶는 데 왜 이렇게 복잡하냐"는 이탈을 부른다.

### 4-8. D-32-9 폐기 경쟁 = 정직한 충돌 (권고 확정)

설계 변경: `trust.seg`에 `pending_succession[]` · 목록/카드에 "후계 충돌" 배지 · 대화창 시스템 라인 · 사용자 선택 시 다른 후보는 `revoked` 처리. 기능: 정상 경우(경쟁 없음)엔 아무것도 안 보인다. 보안: 자동 승자를 두지 않는 것이 유일하게 공격자가 못 이기는 규칙.

---

## 5. 성능 종합 (DR-5 예산 게이트 대조)

| 항목 | 현행 | 변경 후 | 예산 |
|---|---|---|---|
| 세션 수 | 상대 기기 1:1 | 상대 기기 수 × 내 기기 수 + 형제 세션 | 그룹 팬아웃과 같은 성질(ADR-0007 §10) · 유휴 RSS 영향 = 세션당 수 KB |
| 핸드셰이크 CPU | XX | XXpsk3 ≈ 동일 · 판별 +해시 2·AEAD 1 · Ed25519 검증 1회/세션 | 무시 |
| KDF | — | PBKDF2 60k **부팅 1회**(≈30~80ms · 워커) · 암호 변경 시 1회 | 무시 |
| 발신 대역 | 1× | (상대 기기 수 + 형제 수)× — 텍스트는 무시 · **파일은 상대 기기 수만큼**(형제에겐 안 보냄) | UI 명시 |
| 따라잡기 | — | 복귀 시 ≤1MiB/세션 | 설정 |
| 발견 패킷 | ≤512B | 힌트 패킷 별도(+약 60B · 광고 주기의 1/3) | 예산 안 |
| 릴레이 등록 | RID 3 | RID 6 | `MAX_RIDS 8` 안 · 서버 무변경 |
| 저장 | `{peer}.seg` | + `u-{uid}.seg` · `user.key` 100B · trust 레코드 +80B | 무시 |
| 산출물 | 2.84MB | + `ed25519-dalek 2`(추정 +50~80KB · **실측 T-2**) | ≤10MB |
| 의존 | — | +1(퍼미시브 BSD-3 · DR-12 ✅) | 런타임 의존 0 유지 |

---

## 6. 보안 종합 — 위협 모델의 변화

### 6-1. 변하지 않는 것(불변식)

| | 유지 근거 |
|---|---|
| E2E — 릴레이·컨텐츠 서버는 평문을 못 본다(DR-7 · S-3) | PSK·UserKey·마스터 키 전부 종단에만 · 서버는 봉투 |
| 수신 파일은 누가 보냈든 실행하지 않는다(DR-13) | 형제 파일도 격리 · 실체화는 사람 |
| 제로컨피그(DR-1 · S-0) | 핸들·암호는 **옵트인** · 안 넣으면 현행 그대로 |
| 신원 신뢰는 키에(DR-28) | 핀·후계 판정의 앵커 = PeerId |
| 발견 패킷은 힌트(08 §2) | LAN_tag는 힌트 · 판정은 핸드셰이크 |

### 6-2. 새로 생기는 공격면과 방어

```mermaid
flowchart TB
  T1["T-1 온라인 암호 추측\n(XXpsk3 msg3 실패 오라클)"] --> D1["실패 백오프 5회/분→10분 · PBKDF2 60k · 12자 추천"]
  T2["T-2 암호 유출"] --> D2["형제 세션 진입 + UserKey 복제까지 가능\n→ 침해 대응 = UserKey 교체(암호 변경만으론 부족 · UI 분리)"]
  T3["T-3 기기 1대 침해(UserKey 보유)"] --> D3["폐기 경쟁 → 정직한 충돌(D-32-9) · 암호 변경으로 형제 세션 차단"]
  T4["T-4 남이 내 UserId 소속 주장(A-1)"] --> D4["UserHello 서명 검증 · 제시 PeerId ∈ devices · version 단조"]
  T5["T-5 UserHello/Succession 재생"] --> D5["seen_max(version) · 세션 바인딩(Noise 안에서만 수신)"]
  T6["T-6 서버 이름공간 오염(P-3)"] --> D6["서명 챌린지 인증 · PseudoId = HMAC(salt_s, UserId)"]
  T7["T-7 핸들 사칭(같은 이름)"] --> D7["라벨일 뿐 · 이름 충돌 경고 + 지문 표시(현행)"]
  T8["T-8 따라잡기 라인 주입(침해 형제)"] --> D8["형제는 전면 신뢰 = 새 면 아님 · dedup · 상한"]
  T9["T-9 clip 교차(같은 핸들·암호를 두 앱에)"] --> D9["도메인 문자열 분리 → RID·PSK 전부 다름 · 브리지는 별도 옵트인"]
  T10["T-10 암호 = 전 기록 열쇠(컨텐츠 모드)"] --> D10["옵트인 · 시드급 고지 · 서버도 못 연다 · S5 별도 결정"]
```

### 6-3. 약해지는 것(감수) · 강해지는 것

| 약해지는 것 | 강해지는 것 |
|---|---|
| 형제 기기 사이의 신뢰가 **비밀 하나**에 걸린다(종전 = 기기마다 SAS) | 재설치·기기 교체가 사칭 경고를 띄우지 않는다(ADR-0007 §3 ③의 이득이 앞당겨진다) |
| 기기 1대 침해 = 서명 능력(전 기기 복제) | 제3자에 대한 기기 편입 주장이 **서명**으로 검증된다(현행은 아무 검증도 없음) |
| 컨텐츠 모드에서 암호 = 전 기록 열쇠 | 선점 불가 · 후계 연속성 · 서버 이름공간 서명 잠금 |

---

## 7. 실측 항목 (착수 시 게이트)

| # | 항목 | 통과 기준 |
|:--:|---|---|
| T-1 | snow XXpsk3 msg1 AEAD 판별(§3-2) | 인메모리 링크 회귀: psk↔psk 성립 · psk→XX 폴백 · XX→psk 실패 · PSK 불일치 = msg3 실패 · 추가 왕복 0 |
| T-2 | `ed25519-dalek 2` 산출물·트리 | `cargo tree` 중복 0 · 산출물 증가 ≤100KB · lockdown 무관 |
| T-3 | PBKDF2 60k 시간(3-OS) | ≤100ms · 워커 · UI 무정지 |
| T-4 | 2-PC 실기 | Win↔mac 같은 핸들·암호 → "내 기기" 배지 · 승인 0회 파일 · sender copy · 꺼졌다 켜기 = 따라잡기 · 암호 변경 후 재결합 · 침해 대응 후 상대 화면 병합 |
| T-5 | 힌트 없는 LAN(구버전 상대 섞임) | 폴백(§3-4) 성립 · 구버전은 힌트 패킷 폐기 · 미지 Control 태그 무시 |

---

## 8. ✅ 사용자 답변(09-06) — 4문항 전부 권고안 확정 → [46 §9](46-adr-0015-userid-handle-passphrase.md) 표

1. **D-32-3** 파일 승인까지 자동인가(권고) · 승인 창은 남기는가.
2. **D-32-4/5** 1차 = sender copy + 따라잡기 200줄(권고) · 컨텐츠 모드는 실기 후 별도 결정(권고) — 아니면 지금부터 S5까지 한 마일스톤으로.
3. **D-32-6/7** 암호 = `profile.sec` 봉인(권고) · 강도 = 경고만(권고).
4. **D-32-8** UserKey 전 기기 복제(권고 · 아무 기기에서나 추가 · 폐기 경쟁은 충돌 표시) vs ADR-0007 주 기기 전용(경쟁 없음 · 주 기기 켜야 추가 · 복구 시드).
