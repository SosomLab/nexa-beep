$ErrorActionPreference = 'Stop'

# 포터블은 패키지 폴더에만 있다 — choco가 폴더를 지우면 끝(shim 포함).
# ⚠️ 사용자가 만든 서버 키(beepd.key — 별도 경로 권장)는 건드리지 않는다.
Write-Host 'nexa-beepd 제거 — 패키지 폴더만 정리합니다(서버 키는 보존).'
