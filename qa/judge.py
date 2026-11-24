"""스파이크 L3(10만 항목 성능) 판정 — QA 브라이언.

사용: python3 -I judge.py <fd-spike-result-*.tar.gz | out 폴더>

판정 규칙은 측정 전에 고정한다 (qa-d2-linux-sheet.md §부록 C). 앱이 계산한 요약값은 쓰지 않고
perf.log 의 원시 줄에서 직접 계산한다 — 만든 쪽 숫자를 그대로 믿지 않는다.

perf.log 줄 (탭 구분 5칸, C#판 PerformanceLog 와 같은 모양):
  시각  지점  ms  예산ms  ok|over
  지점 = ColdStart | WindowShown | FirstItem | Frame:details | Frame:icons
"""
import math
import statistics
import sys
import tarfile
import tempfile
import unicodedata
from pathlib import Path

FIRST_ITEM_BUDGET = 150.0
FRAME_BUDGET = 16.7
FRAME_JITTER = 2.0          # 타이머 오차 허용 — 경계 판정에만 쓴다
DROPPED_MS = 25.0           # 60Hz 한 칸 반을 넘으면 놓친 프레임으로 본다
DROPPED_MAX_RATIO = 0.05
MIN_FIRST_ITEM = 10
MIN_FRAMES = 300            # 보기 하나당 (10초 자동 스크롤이면 60Hz 에서 약 600)


def p_nearest(values, p):
    v = sorted(values)
    k = max(1, math.ceil(p * len(v)))
    return v[k - 1]


def load(out_dir: Path, cand: str):
    path = out_dir / cand / "perf.log"
    rows = {}
    bad = 0
    if not path.exists():
        return None, 0
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        parts = line.split("\t")
        if len(parts) != 5:
            bad += 1
            continue
        try:
            rows.setdefault(parts[1], []).append(float(parts[2]))
        except ValueError:
            bad += 1
    return rows, bad


def judge_frames(name, xs):
    if len(xs) < MIN_FRAMES:
        return "FAIL", f"{name}: 프레임 {len(xs)}개 < {MIN_FRAMES} (측정 부족)"
    p95 = p_nearest(xs, 0.95)
    dropped = sum(1 for x in xs if x > DROPPED_MS) / len(xs)
    detail = (f"{name}: n={len(xs)} p50={statistics.median(xs):.1f} p95={p95:.1f} "
              f"max={max(xs):.1f} 놓친프레임(>{DROPPED_MS:.0f}ms)={dropped:.1%}")
    if p95 <= FRAME_BUDGET:
        return "PASS", detail
    if p95 <= FRAME_BUDGET + FRAME_JITTER and dropped <= DROPPED_MAX_RATIO:
        return "BORDER", detail + " — 오차 범위, 사람 체감(성-2)과 함께 본다"
    return "FAIL", detail


# 손 확인(끌-·한-)이 끝난 뒤 파일 상태로 다시 대조한다. 대표님 ✅/❌와 어긋나면 재확인 대상이다.
# (기대 상태, 확인표 번호, 설명). 경로는 시험 폴더 기준 끝부분으로 맞춘다.
FILE_EXPECT = [
    (True,  "끌-1", "끌어놓기/받는폴더/밖에서-온-파일.txt"),
    (True,  "끌-1", "파일관리자쪽/밖에서-온-파일.txt"),          # 기본 = 복사 → 원본 남음
    (True,  "끌-2", "끌어놓기/밖에서-온-폴더"),
    (False, "끌-2", "파일관리자쪽/밖에서-온-폴더"),              # Shift = 이동 → 원본 사라짐
    (True,  "끌-3", "파일관리자쪽/보낼-메모.txt"),
    (True,  "끌-3", "끌어놓기/보낼-메모.txt"),                    # 기본 = 복사
    (True,  "끌-4", "파일관리자쪽/옮겨질-파일.txt"),
    (False, "끌-4", "끌어놓기/옮겨질-파일.txt"),                  # Shift = 이동
    (True,  "한-1", "한글입력/회의.txt"),
    (False, "한-1", "한글입력/가방.txt"),
    (False, "한-1", "한글입력/회의의.txt"),
    (False, "한-1", "한글입력/회.txt"),
    (True,  "한-2", "한글입력/문서함.txt"),
    (True,  "한-3", "한글입력/하늘.txt"),
]


def check_files(out_dir: Path, cand: str):
    f = out_dir / cand / "after-open-files.txt"
    if not f.exists():
        print("  (손 확인 파일 상태 없음 — open 을 안 했거나 기록 실패)")
        return
    have = {unicodedata.normalize("NFC", line.strip()) for line in f.read_text(encoding="utf-8", errors="replace").splitlines()}
    print("  손 확인 뒤 파일 상태:")
    for expected, tag, rel in FILE_EXPECT:
        rel_n = unicodedata.normalize("NFC", rel)
        present = any(h.endswith("/" + rel_n) for h in have)
        ok = present == expected
        print(f"    {'✅' if ok else '❌'} {tag} {rel} {'있어야' if expected else '없어야'} 함 → {'있음' if present else '없음'}")
    copied = [h for h in have if h.endswith("/보낼-메모.txt") and "/한글입력/" in h]
    print(f"    {'✅' if copied else '❌'} 끌-6 칸 사이 복사본(한글입력 쪽) → {'있음' if copied else '없음'}")


def judge(out_dir: Path):
    env = out_dir / "env.txt"
    if env.exists():
        print("── 기기")
        for line in env.read_text(encoding="utf-8", errors="replace").splitlines():
            if line.startswith(("PRETTY_NAME", "session", "big_count", "test_fs", "kernel")) or "*" in line:
                print("  " + line.strip())
    verdicts = {}
    for cand in ("A", "B"):
        rows, bad = load(out_dir, cand)
        print(f"\n── 후보 {cand}")
        if rows is None:
            print("  perf.log 없음 → L3 FAIL (측정 불가)")
            verdicts[cand] = "FAIL"
            check_files(out_dir, cand)
            continue
        if bad:
            print(f"  ⚠ 형식이 틀린 줄 {bad}개 (제외)")
        parts = []
        fi = rows.get("FirstItem", [])
        if len(fi) < MIN_FIRST_ITEM:
            parts.append(("FAIL", f"FirstItem: {len(fi)}회 < {MIN_FIRST_ITEM}"))
        else:
            p95 = p_nearest(fi, 0.95)
            parts.append(("PASS" if p95 <= FIRST_ITEM_BUDGET else "FAIL",
                          f"FirstItem: n={len(fi)} 중앙값={statistics.median(fi):.0f} p95={p95:.0f} (예산 {FIRST_ITEM_BUDGET:.0f})"))
        for view in ("details", "icons"):
            parts.append(judge_frames(f"Frame:{view}", rows.get(f"Frame:{view}", [])))
        for pt, budget in (("ColdStart", 1500), ("WindowShown", 100)):
            xs = rows.get(pt, [])
            if xs:
                print(f"  (참고) {pt}: {', '.join(f'{x:.0f}' for x in xs)} ms (예산 {budget})")
        for v, d in parts:
            print(f"  [{v}] {d}")
        kinds = [v for v, _ in parts]
        verdicts[cand] = "FAIL" if "FAIL" in kinds else ("BORDER" if "BORDER" in kinds else "PASS")
        check_files(out_dir, cand)
        ev = out_dir / cand / "events.log"
        if ev.exists():
            n = sum(1 for _ in ev.open(encoding="utf-8", errors="replace"))
            print(f"  events.log {n}줄")
    print("\n  ※ 프레임 측정 방식이 다르다 — A: 웹뷰 프레임 시각(rAF) 간격, B: 앱이 그리는 순간을 직접 잼."
          "\n    후보별 절대 기준 판정은 그대로, A·B 프레임 수치 직접 비교(결정 규칙 3)는 참고로만 쓴다.")
    print("\n── L3 판정: " + " · ".join(f"{c}={v}" for c, v in verdicts.items()))
    return verdicts


def main():
    src = Path(sys.argv[1])
    if src.is_file():
        with tempfile.TemporaryDirectory() as tmp, tarfile.open(src) as tf:
            tf.extractall(tmp, filter="data")
            judge(Path(tmp) / "out")
    else:
        judge(src)


if __name__ == "__main__":
    main()
