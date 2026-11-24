#!/usr/bin/env bash
# AppImage 가 **호스트에서 가져다 쓰는** 라이브러리 중 지금 기계에 없는 것을 적는다.
# (GL·EGL 같은 드라이버 라이브러리는 AppImage 에 넣지 않는 것이 규칙이라 호스트에 있어야 한다.)
# 사용: appimage-deps.sh <AppImage>...   → stdout: "<앱 이름>\t<없는 soname>" 줄들. 실패로 끝나지 않는다.
set -uo pipefail
for f in "$@"; do
  name=$(basename "$f")
  d=$(mktemp -d)
  (cd "$d" && "$f" --appimage-extract >/dev/null 2>&1)
  root="$d/squashfs-root"
  libdirs=$(find "$root" -type f -name '*.so*' -printf '%h\n' | sort -u | tr '\n' ':')
  find "$root" -type f \( -name '*.so*' -o -perm -u+x \) -print0 |
    while IFS= read -r -d '' bin; do
      file -b "$bin" | grep -q ELF || continue
      LD_LIBRARY_PATH="$libdirs" ldd "$bin" 2>/dev/null | awk '/not found/ {print $1}'
    done | sort -u | sed "s|^|$name\t|"
  rm -rf "$d"
done
