#!/usr/bin/env bash
# 시험 앱 B (Slint · Qt 백엔드) AppImage 만들기 — CI(ubuntu-22.04) 전용.
#
# 전제 (CI 앞 단계):
#   sudo apt-get install -y qt6-base-dev qt6-wayland libqt6svg6 librsvg2-bin file desktop-file-utils
#   QMAKE=/usr/lib/qt6/bin/qmake cargo build --release -p fd-slint-shell --no-default-features --features qt
#
# 사용:
#   apps/slint-shell/packaging/build-appimage.sh <출력 폴더> [바이너리 경로]
#   SMOKE=1 …  → 만든 AppImage 를 QT_QPA_PLATFORM=offscreen 으로 자동 측정까지 돌려 본다 (화면 없는 CI 용, 숫자는 추세만)
#
# 결과: <출력 폴더>/flex-dir-spike-slint-x86_64.AppImage  (fd-check.sh 는 이름에 slint 가 든 .AppImage 를 찾는다)
# fd-check.sh 는 APPIMAGE_EXTRACT_AND_RUN=1 로 띄운다 — 이 스크립트의 SMOKE 도 같은 방식으로 띄운다.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../../.." && pwd)"
APP=flex-dir-spike-slint
ARCH="${ARCH:-x86_64}"
OUT="$(mkdir -p "${1:-$REPO/dist}" && cd "${1:-$REPO/dist}" && pwd)"
BIN="${2:-${CARGO_TARGET_DIR:-$REPO/target}/release/$APP}"
export QMAKE="${QMAKE:-/usr/lib/qt6/bin/qmake}"
WORK="${WORK_DIR:-$(mktemp -d)}"
APPDIR="$WORK/AppDir"
TOOLS="${TOOLS_DIR:-$WORK/tools}"
OUTFILE="$OUT/$APP-$ARCH.AppImage"

say()  { printf '[build-appimage] %s\n' "$*"; }
fail() { printf '[build-appimage] ✗ %s\n' "$*" >&2; exit 1; }

[ -x "$BIN" ] || fail "바이너리가 없습니다: $BIN (cargo build --release -p fd-slint-shell --no-default-features --features qt)"
[ -x "$QMAKE" ] || fail "qmake 가 없습니다: $QMAKE (qt6-base-dev)"

# Qt 를 못 찾고 빌드되면 i-slint-backend-qt 가 빈 껍데기(no_qt)로 컴파일되고 경고만 남는다 → 실행 때 창이 안 뜬다.
# 진짜 Qt 백엔드면 libQt6Widgets 를 링크한다. 여기서 막는다.
ldd "$BIN" | grep -q 'libQt6Widgets' || fail "바이너리가 Qt6 를 링크하지 않습니다 — Qt 백엔드가 빠진 빌드입니다 (QMAKE 확인)"

# ── 도구 (linuxdeploy + Qt 플러그인). CI 러너에는 FUSE 가 없으니 풀어서 실행한다.
export APPIMAGE_EXTRACT_AND_RUN=1
mkdir -p "$TOOLS"
fetch() { # $1=url $2=file
  [ -s "$TOOLS/$2" ] || curl -fsSL --retry 3 -o "$TOOLS/$2" "$1"
  chmod +x "$TOOLS/$2"
}
fetch "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-$ARCH.AppImage" "linuxdeploy-$ARCH.AppImage"
fetch "https://github.com/linuxdeploy/linuxdeploy-plugin-qt/releases/download/continuous/linuxdeploy-plugin-qt-$ARCH.AppImage" "linuxdeploy-plugin-qt-$ARCH.AppImage"
export PATH="$TOOLS:$PATH"   # linuxdeploy 가 --plugin qt 를 PATH 에서 찾는다

# ── AppDir
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/$APP"
install -m 755 "$BIN" "$APPDIR/usr/bin/$APP"
# 글꼴·아이콘은 바이너리에 들어 있다. 원본 사본과 OFL 라이선스 전문을 같이 싣는다 (OFL 재배포 조건).
cp -r "$REPO/assets/fonts" "$APPDIR/usr/share/$APP/fonts"
cp -r "$REPO/assets/icons" "$APPDIR/usr/share/$APP/icons"

cat > "$APPDIR/usr/share/applications/$APP.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=flex-dir spike B (Slint)
Comment=GUI stack spike — Slint (Qt backend)
Exec=$APP %F
Icon=$APP
Terminal=false
Categories=Utility;
EOF

# 256×256 png 아이콘 (folder.svg 에서). rsvg-convert 가 없으면 단색 png 를 직접 만든다.
ICON="$WORK/$APP.png"
if command -v rsvg-convert >/dev/null 2>&1; then
  rsvg-convert -w 256 -h 256 "$REPO/assets/icons/folder.svg" -o "$ICON"
else
  say "rsvg-convert 없음 → 단색 아이콘"
  python3 - "$ICON" <<'PY'
import struct, sys, zlib
w = h = 256
row = b"\x00" + bytes([0xF5, 0xC4, 0x51, 0xFF]) * w
def chunk(t, d):
    return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(row * h, 9)) + chunk(b"IEND", b"")
open(sys.argv[1], "wb").write(png)
PY
fi

# ── 배포. Wayland 플랫폼 플러그인을 같이 싣는다 (기본은 xcb 만). offscreen 은 SMOKE 용.
export EXTRA_PLATFORM_PLUGINS="libqwayland-egl.so;libqwayland-generic.so;libqoffscreen.so"
export LDAI_OUTPUT="$OUTFILE" OUTPUT="$OUTFILE"   # linuxdeploy-plugin-appimage 출력 이름 (새·옛 변수)
rm -f "$OUTFILE"
(
  cd "$WORK"
  "linuxdeploy-$ARCH.AppImage" \
    --appdir "$APPDIR" \
    --executable "$APPDIR/usr/bin/$APP" \
    --desktop-file "$APPDIR/usr/share/applications/$APP.desktop" \
    --icon-file "$ICON" \
    --custom-apprun "$HERE/AppRun" \
    --plugin qt \
    --output appimage
)
[ -s "$OUTFILE" ] || fail "AppImage 가 만들어지지 않았습니다: $OUTFILE"

# 무엇이 실렸는지 기록 (입력기·Wayland 판정 근거)
say "실린 Qt 플러그인:"
find "$APPDIR/usr/plugins" -name '*.so' | sed "s|$APPDIR/||" | sort | sed 's/^/  /'
ls "$APPDIR/usr/plugins/platforminputcontexts/" 2>/dev/null | grep -q ibus || say "⚠ ibus 입력 플러그인이 없습니다 — 한글 입력(L2)이 안 될 수 있습니다"
say "완성: $OUTFILE ($(du -h "$OUTFILE" | cut -f1))"

# ── 선택: 화면 없이 자동 측정이 끝까지 도는지 (exit 0 · FirstItem 10줄 · 프레임 줄)
if [ "${SMOKE:-0}" = "1" ]; then
  S="$WORK/smoke"; rm -rf "$S"; mkdir -p "$S/big" "$S/out"
  (cd "$S/big" && seq 1 5000 | sed 's/^/img/; s/$/.jpg/' | xargs touch)
  rc=0
  env APPIMAGE_EXTRACT_AND_RUN=1 QT_QPA_PLATFORM=offscreen FD_SPIKE_PERF_AUTO=1 FD_SPIKE_CANDIDATE=B \
      FD_SPIKE_OUT="$S/out" timeout 300 "$OUTFILE" "$S/big" >"$S/stdout.txt" 2>&1 || rc=$?
  fi_n=$(grep -c $'\tFirstItem\t' "$S/out/perf.log" 2>/dev/null || true)
  fr_n=$(grep -c $'\tFrame:' "$S/out/perf.log" 2>/dev/null || true)
  say "SMOKE: exit=$rc FirstItem=$fi_n Frame=$fr_n"
  grep -E $'\t(backend|perf-method|fonts|fonts-error|env)\t' "$S/out/events.log" | cut -f2- | sed 's/^/  /' || true
  [ "$rc" -eq 0 ] && [ "${fi_n:-0}" -eq 10 ] && [ "${fr_n:-0}" -gt 0 ] || { tail -n 40 "$S/stdout.txt" >&2; fail "SMOKE 실패"; }
fi
