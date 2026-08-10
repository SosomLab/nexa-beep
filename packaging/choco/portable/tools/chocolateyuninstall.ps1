$ErrorActionPreference = 'Stop'

# 포터블은 패키지 폴더에만 있다 — choco가 폴더를 지우면 끝이고, shim도 함께 사라진다.
# ⚠️ 사용자가 실행 파일 옆에 만든 것(DR-4 포터블 영속물)은 건드리지 않는다.
Write-Host 'Nexa Beep (Portable) 제거 — 패키지 폴더만 정리합니다.'
