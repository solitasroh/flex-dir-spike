#!/usr/bin/env bash
# fd-check.sh 는 앱이 죽어도 단계를 실패시키지 않는다 (대표님용 안내 스크립트라 의도된 동작).
# CI 에서는 여기서 막는다. 하나라도 어기면 exit 1 → Release 단계가 돌지 않는다.
#   - 종료 코드 0 (stdout-perf.txt 의 exit=)
#   - ColdStart 1줄, FirstItem 정확히 10줄
#   - Frame:details · Frame:icons 각각 100줄 이상
#   - perf.log 의 모든 줄이 탭 5칸
# 결과 요약은 $1 파일(릴리스 노트용)과 GITHUB_STEP_SUMMARY 에 쓴다.
set -uo pipefail
OUT="${FD_OUT:-$HOME/fd-spike-test/out}"
SUMMARY="${1:-e2e-summary.md}"
fail=0
{
  echo "| 앱 | 종료 코드 | ColdStart 줄 | FirstItem 줄 | Frame:details 줄 | Frame:icons 줄 | 5칸 아닌 줄 | FirstItem 중앙값·최댓값(ms) | 결과 |"
  echo "|---|---|---|---|---|---|---|---|---|"
} > "$SUMMARY"
for c in A B; do
  log="$OUT/$c/perf.log"; so="$OUT/$c/stdout-perf.txt"
  rc=$(grep -oE '^exit=[0-9]+' "$so" 2>/dev/null | tail -n 1 | cut -d= -f2); rc=${rc:-없음}
  cs_n=0; fi_n=0; fd_n=0; ic_n=0; bad=0; fi_stat="-"
  if [ -f "$log" ]; then
    cs_n=$(grep -c $'\tColdStart\t' "$log" || true)
    fi_n=$(grep -c $'\tFirstItem\t' "$log" || true)
    fd_n=$(grep -c $'\tFrame:details\t' "$log" || true)
    ic_n=$(grep -c $'\tFrame:icons\t' "$log" || true)
    bad=$(awk -F'\t' 'NF != 5' "$log" | wc -l)
    fi_stat=$(awk -F'\t' '$2=="FirstItem"{print $3}' "$log" | sort -n | awk '{a[NR]=$1} END{if(NR) printf "%.0f / %.0f", a[int((NR+1)/2)], a[NR]}')
  fi
  ok=1
  [ "$rc" = "0" ] || ok=0
  [ "$cs_n" -eq 1 ] || ok=0
  [ "$fi_n" -eq 10 ] || ok=0
  [ "$fd_n" -ge 100 ] || ok=0
  [ "$ic_n" -ge 100 ] || ok=0
  [ "$bad" -eq 0 ] || ok=0
  verdict=$([ $ok -eq 1 ] && echo "통과" || echo "**실패**")
  echo "| $c | $rc | $cs_n | $fi_n | $fd_n | $ic_n | $bad | ${fi_stat:--} | $verdict |" >> "$SUMMARY"
  if [ $ok -ne 1 ]; then
    fail=1
    echo "::error::시험 앱 $c: exit=$rc ColdStart=$cs_n FirstItem=$fi_n Frame:details=$fd_n Frame:icons=$ic_n 5칸아님=$bad"
    echo "----- $c stdout-perf.txt (끝 40줄)"; tail -n 40 "$so" 2>/dev/null || echo "(없음)"
  fi
done
echo "" >> "$SUMMARY"
echo "가상 화면(Xvfb, mesa 소프트웨어 렌더링) 숫자 — 도는지와 추세만 본다. 판정은 대표님 기기에서 한다." >> "$SUMMARY"
cat "$SUMMARY"
[ -n "${GITHUB_STEP_SUMMARY:-}" ] && cat "$SUMMARY" >> "$GITHUB_STEP_SUMMARY"
exit $fail
