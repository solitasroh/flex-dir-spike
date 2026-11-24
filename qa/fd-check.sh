#!/usr/bin/env bash
# flex-dir 스파이크 — 대표님 Linux 기기 확인 도우미 (QA 브라이언, 2026-11-25)
#
#   bash fd-check.sh prepare     시험 폴더 만들기 + 기기 정보 기록 (1분)
#   bash fd-check.sh perf A      시험 앱 A 성능 자동 측정 (앱이 스스로 돌고 꺼진다)
#   bash fd-check.sh perf B
#   bash fd-check.sh open B      시험 앱 B + 파일 관리자 창을 연다 (손으로 확인할 때)
#   bash fd-check.sh open A
#   bash fd-check.sh collect     결과를 파일 하나로 묶는다 → 슬랙에 올린다
#   bash fd-check.sh cleanup     시험 폴더를 지운다 (마지막에 한 번)
#
# 안전 규칙: 이 스크립트는 시험 폴더(~/fd-spike-test) 밖의 파일을 만들거나 지우지 않는다.
#           예외는 collect 가 홈 폴더에 남기는 결과 파일 하나뿐이다.
#           root 권한이 필요한 일은 하지 않는다.

set -euo pipefail

ROOT="${FD_TEST_ROOT:-$HOME/fd-spike-test}"
OUT="$ROOT/out"
MARK="$ROOT/.fd-spike-test"
BIG="$ROOT/큰폴더-10만"
DND="$ROOT/끌어놓기"
IME="$ROOT/한글입력"
FM_SIDE="$ROOT/파일관리자쪽"
EXPECTED_BIG=100000
SCRIPT_VERSION="2026-11-25.2"

say()  { printf '%s\n' "$*"; }
fail() { printf '\n× %s\n' "$*" >&2; exit 1; }

need_root_dir() {
  [ -f "$MARK" ] || fail "시험 폴더가 없습니다. 먼저 'bash fd-check.sh prepare' 를 실행해 주세요."
}

# ── 앱 찾기 ─────────────────────────────────────────────────────────
# FD_APP_A / FD_APP_B 로 직접 줄 수도 있다. 안 주면 다운로드 폴더에서 이름으로 찾는다.
find_app() {
  local which="$1" var="FD_APP_$1" pat f
  if [ -n "${!var:-}" ]; then
    [ -e "${!var}" ] || fail "$var 경로에 파일이 없습니다: ${!var}"
    printf '%s' "${!var}"; return
  fi
  case "$which" in
    A) pat='*[Tt]auri*' ;;
    B) pat='*[Ss]lint*' ;;
    *) fail "A 또는 B 를 적어 주세요." ;;
  esac
  f=$(find "$HOME/Downloads" "$HOME/다운로드" "$HOME" -maxdepth 1 -type f -name "$pat" -name '*.AppImage' 2>/dev/null | sort | tail -n 1 || true)
  [ -n "$f" ] || fail "시험 앱 $which 파일(.AppImage)을 다운로드 폴더에서 찾지 못했습니다. 받은 파일 이름에 tauri(A) / slint(B) 가 들어 있는지 봐 주세요."
  chmod u+x "$f"
  printf '%s' "$f"
}

# AppImage 는 libfuse2 가 없는 배포판(Ubuntu 22.04+)에서 바로 안 뜬다. 풀어서 실행하게 한다.
# 주의: 앱 경로는 호출하는 쪽이 먼저 찾아서 넘긴다. 여기서 찾으면 '|| rc=$?' 문맥이라
#       set -e 가 꺼져 있어 못 찾아도 멈추지 않는다 (자체 시험에서 재현된 결함).
run_app() {  # $1=A|B  $2=앱 경로  $3=앞에 붙일 것(환경변수·timeout, 공백 구분, 없으면 "")  나머지=폴더(왼쪽, 오른쪽)
  local which="$1" app="$2" pre="$3"; shift 3
  say "→ 시험 앱 $which: $(basename "$app")" >&2
  # shellcheck disable=SC2086
  env APPIMAGE_EXTRACT_AND_RUN=1 FD_SPIKE_OUT="$OUT/$which" FD_SPIKE_CANDIDATE="$which" $pre "$app" "$@"
}

# ── 기기 정보 ───────────────────────────────────────────────────────
capture_env() {
  {
    echo "script=$SCRIPT_VERSION"
    echo "when=$(date -Iseconds)"
    grep -E '^(PRETTY_NAME|VERSION_ID)=' /etc/os-release || true
    echo "session=${XDG_SESSION_TYPE:-?} desktop=${XDG_CURRENT_DESKTOP:-?} wayland_display=${WAYLAND_DISPLAY:-} display=${DISPLAY:-}"
    echo "im: GTK_IM_MODULE=${GTK_IM_MODULE:-} QT_IM_MODULE=${QT_IM_MODULE:-} XMODIFIERS=${XMODIFIERS:-}"
    echo "im_process:"; pgrep -a 'fcitx5|ibus-daemon|fcitx' || echo "  (입력기 프로세스 없음)"
    echo "gpu:"; (command -v lspci >/dev/null && lspci | grep -iE 'vga|3d|display') || echo "  (lspci 없음)"
    (command -v glxinfo >/dev/null && glxinfo -B 2>/dev/null | grep -E 'renderer string|version string') || true
    (command -v nvidia-smi >/dev/null && nvidia-smi --query-gpu=name,driver_version --format=csv,noheader) || true
    echo "screen:"; (command -v xrandr >/dev/null && xrandr --current 2>/dev/null | grep -E '\*|connected primary') || echo "  (xrandr 없음)"
    (command -v gsettings >/dev/null && echo "gnome_scale=$(gsettings get org.gnome.desktop.interface scaling-factor 2>/dev/null) text_scale=$(gsettings get org.gnome.desktop.interface text-scaling-factor 2>/dev/null)") || true
    echo "kernel=$(uname -r) cpus=$(nproc)"
    free -m | sed -n '1,2p'
    echo "test_fs=$(findmnt -no FSTYPE -T "$ROOT" 2>/dev/null || echo ?) $(df -h "$ROOT" | tail -n 1)"
    echo "webkit:"; (ls /usr/lib/*/libwebkit2gtk-4.1.so* /usr/lib/libwebkit2gtk-4.1.so* 2>/dev/null | head -n 2) || echo "  (webkit2gtk-4.1 시스템 라이브러리 없음)"
    echo "qt:"; (ls /usr/lib/*/libQt6Widgets.so* /usr/lib/libQt6Widgets.so* 2>/dev/null | head -n 1) || echo "  (Qt6 시스템 라이브러리 없음)"
  } > "$OUT/env.txt" 2>&1
}

# ── 작은 시험 폴더 (앱을 바꿀 때마다 새로 만든다) ───────────────────
make_small() {
  rm -rf -- "$DND" "$IME" "$FM_SIDE"
  mkdir -p "$DND/받는폴더" "$IME/한글폴더" "$FM_SIDE/밖에서-온-폴더"
  printf '복사 시험용\n' > "$DND/보낼-메모.txt"
  printf 'move test\n'   > "$DND/옮겨질-파일.txt"
  printf 'report\n'      > "$DND/report.txt"
  printf '파일 관리자에서 끌어온 파일\n' > "$FM_SIDE/밖에서-온-파일.txt"
  printf 'a\n' > "$FM_SIDE/밖에서-온-폴더/안쪽.txt"
  # 한글 입력 확인용: NFC 이름, NFD 이름(맥에서 흔한 형식), 대소문자 쌍, 한자·이모지·아랍어(엘레나 폰트 항목용)
  : > "$IME/가방.txt"
  : > "$IME/문서함.txt"
  : > "$IME/하늘.txt"
  : > "$IME/한글.txt"                                                     # NFC
  : > "$IME/$(printf '\xe1\x84\x92\xe1\x85\xa1\xe1\x86\xab\xe1\x84\x80\xe1\x85\xb3\xe1\x86\xaf\x32').txt"  # NFD "한글2"
  : > "$IME/apple.txt"
  : > "$IME/Apple.txt"
  : > "$IME/漢字_日本語.txt"
  : > "$IME/이모지🎉.txt"
  : > "$IME/ملف عربي.txt"
  # ── 엘레나 삽입분 (spike-d2-design-items.md A-1, 그대로) ──
  : > "$IME/ghost.txt"   # 한글이 영문 키로 새는지 잡는 덫: '한글' = g k s r m f
  printf '주소줄 시험\n' > "$IME/한글폴더/여기까지오면통과.txt"
  # 글꼴 D3(숫자 열 정렬)용 — 크기·날짜가 다 0이면 숫자 열을 볼 수 없다
  truncate -s 1232896  "$IME/report_final_v2 (1).pdf";  touch -d "2026-11-23 18:05" "$IME/report_final_v2 (1).pdf"
  truncate -s 90636288 "$IME/디자인시스템 토큰 v3.fig"; touch -d "2026-11-20 09:11" "$IME/디자인시스템 토큰 v3.fig"
  truncate -s 18432    "$IME/2026-11-24 회의록.docx";   touch -d "2026-11-24 10:41" "$IME/2026-11-24 회의록.docx"
}

# ── 10만 항목 폴더 (한 번만 만든다) ─────────────────────────────────
make_big() {
  rm -rf -- "$BIG"
  mkdir -p "$BIG"
  local long; long=$(printf 'long_name_%.0s' $(seq 1 18))   # 180자
  (
    cd "$BIG"
    {
      for i in $(seq 1 40000);  do printf 'img%d.jpg\0' "$i"; done                 # 자연 정렬 (img2 < img10)
      for i in $(seq 1 30000);  do printf '문서_%05d.txt\0' "$i"; done             # 한글 NFC
      for i in $(seq 1 10000);  do printf '\xe1\x84\x92\xe1\x85\xa1\xe1\x86\xab\xe1\x84\x80\xe1\x85\xb3\xe1\x86\xaf_%05d.md\0' "$i"; done  # 한글 NFD
      for i in $(seq 1 5000);   do printf 'Case%04d.txt\0case%04d.txt\0' "$i" "$i"; done  # 대소문자만 다른 쌍
      for i in $(seq 1 9000);   do printf '%s_%04d.dat\0' "$long" "$i"; done        # 긴 이름
    } | xargs -0 touch --
    for i in $(seq 1 1000); do printf '폴더%04d\0' "$i"; done | xargs -0 mkdir --
  )
  local n; n=$(find "$BIG" -mindepth 1 -maxdepth 1 | wc -l)
  echo "big_count=$n" >> "$OUT/env.txt"
  if [ "$n" -ne "$EXPECTED_BIG" ]; then
    say "⚠ 10만 폴더 항목 수가 $n 개입니다 ($EXPECTED_BIG 개여야 함). 이 디스크가 대소문자를 구분하지 않으면 이렇게 됩니다 — 그대로 진행하고 슬랙에 숫자만 알려 주세요."
  fi
}

cmd_prepare() {
  if [ -e "$ROOT" ] && [ ! -f "$MARK" ]; then
    fail "$ROOT 가 이미 있고 시험 폴더 표시가 없습니다. 대표님 파일일 수 있어 건드리지 않습니다. FD_TEST_ROOT=다른경로 로 다시 실행해 주세요."
  fi
  mkdir -p "$OUT"
  : > "$MARK"
  local t0; t0=$(date +%s)
  capture_env
  say "기기 정보를 적었습니다."
  make_small
  say "10만 항목 폴더를 만드는 중입니다 (1분 안팎)…"
  make_big
  say "준비 끝 ($(( $(date +%s) - t0 ))초). 시험 폴더: $ROOT"
  say "기기 정보 요약:"
  grep -E '^PRETTY_NAME|^session' "$OUT/env.txt" | sed 's/^/  /'
  grep -A1 -E '^(im_process|gpu):' "$OUT/env.txt" | grep -v -E '^(--|im_process:|gpu:)' | sed 's/^ */  /'
}

cmd_perf() {
  need_root_dir
  local which="${1:-}"; [ -n "$which" ] || fail "A 또는 B 를 적어 주세요. 예: bash fd-check.sh perf A"
  local app; app=$(find_app "$which")
  say "앱이 스스로 큰 폴더를 10번 열고, 두 가지 보기에서 자동으로 스크롤한 뒤 꺼집니다 (2분 안쪽)."
  say "보고만 계시면 됩니다. 멈추거나 하얗게 비는 화면이 보이면 기억해 두세요."
  local rc=0 p="$OUT/$which/perf.log" fi=0 fr=0
  mkdir -p "$OUT/$which"
  # 이전 측정이 남아 있으면 옮겨 둔다. 섞이면 앱이 안 떠도 '저장됨' 으로 보인다 (자체 시험에서 재현된 결함).
  [ -f "$p" ] && mv -- "$p" "$OUT/$which/perf.prev-$(date +%H%M%S).log"
  run_app "$which" "$app" "FD_SPIKE_PERF_AUTO=1 timeout 300" "$BIG" >"$OUT/$which/stdout-perf.txt" 2>&1 || rc=$?
  echo "exit=$rc" >> "$OUT/$which/stdout-perf.txt"
  if [ -f "$p" ]; then
    fi=$(grep -c $'\tFirstItem\t' "$p" || true)
    fr=$(grep -c $'\tFrame:' "$p" || true)
  fi
  if [ "$rc" -eq 124 ]; then
    say "× 시험 앱 $which 이 5분 안에 끝나지 않아 멈췄습니다. 확인표 성-1 에 × 로 적어 주세요."
  elif [ "$rc" -ne 0 ]; then
    say "× 시험 앱 $which 이 오류로 끝났습니다 (종료 코드 $rc). 확인표 성-1 에 × 로 적어 주세요."
  fi
  if [ "$rc" -ne 0 ] || [ "$fi" -lt 10 ] || [ "$fr" -lt 100 ]; then
    say "× 측정 기록이 모자랍니다 (폴더 열기 ${fi}/10, 화면 프레임 ${fr}개). 성-1 에 × 로 적어 주세요."
  else
    say "○ 측정 기록 저장됨 (폴더 열기 ${fi}회, 화면 프레임 ${fr}개). 판정은 브라이언이 합니다."
  fi
}

cmd_open() {
  need_root_dir
  local which="${1:-}"; [ -n "$which" ] || fail "A 또는 B 를 적어 주세요. 예: bash fd-check.sh open B"
  local app; app=$(find_app "$which")
  make_small                 # 앞 앱이 옮기거나 바꾼 파일을 원래대로 되돌린다
  say "파일 관리자 창(왼쪽에 두세요)과 시험 앱 $which 창이 열립니다."
  say "다 보셨으면 시험 앱 창을 닫으세요. 그러면 이 화면으로 돌아옵니다."
  (xdg-open "$FM_SIDE" >/dev/null 2>&1 &) || true
  local rc=0
  mkdir -p "$OUT/$which"
  run_app "$which" "$app" "" "$DND" "$IME" >"$OUT/$which/stdout-open.txt" 2>&1 || rc=$?
  [ "$rc" -eq 0 ] || say "× 시험 앱 $which 이 오류로 끝났습니다 (종료 코드 $rc). 확인표에 메모해 주세요."
  echo "exit=$rc" >> "$OUT/$which/stdout-open.txt"
  # 확인 뒤 상태를 남긴다 — 무엇이 어디로 갔는지 브라이언이 로그와 대조한다
  find "$DND" "$IME" "$FM_SIDE" -mindepth 1 | sort > "$OUT/$which/after-open-files.txt"
  say "시험 앱 $which 확인 기록을 저장했습니다."
}

cmd_collect() {
  need_root_dir
  local f="$HOME/fd-spike-result-$(date +%m%d-%H%M).tar.gz"
  tar -czf "$f" -C "$ROOT" out
  say "결과 파일: $f ($(du -h "$f" | cut -f1))"
  say "이 파일을 슬랙 스레드에 올려 주세요."
}

cmd_cleanup() {
  need_root_dir
  say "시험 폴더 $ROOT 를 지웁니다 (10만 개 빈 파일 포함). 결과 파일(~/fd-spike-result-*.tar.gz)은 남습니다."
  read -r -p "지울까요? [y/N] " ans
  [ "$ans" = "y" ] || [ "$ans" = "Y" ] || { say "지우지 않았습니다."; return; }
  rm -rf -- "$ROOT"
  say "지웠습니다."
}

case "${1:-}" in
  prepare) cmd_prepare ;;
  perf)    cmd_perf "${2:-}" ;;
  open)    cmd_open "${2:-}" ;;
  collect) cmd_collect ;;
  cleanup) cmd_cleanup ;;
  *) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//' ;;
esac
