# qa/ — 브라이언 도구 사본

정본은 `flex-dir-rust-migration/planning/qa-tools/` 이다. 여기 사본은 CI(`e2e-linux`)가 시험 앱을 브라이언 스크립트로
끝까지 돌려 보는 데만 쓴다. `SHA256SUMS` 가 확인표 부록(`spike-d2-linux-sheet.md`)의 해시와 같아야 하고,
CI 가 매번 `sha256sum -c` 로 확인한다. 정본이 바뀌면 사본과 해시를 같이 바꾼다.

- `fd-check.sh` v2026-11-25.3 (`gfx` 그래픽 라이브러리 점검 추가) · `judge.py`
- 옛 v2026-11-25.2 (`d2d5676e…`)는 쓰지 않는다.
