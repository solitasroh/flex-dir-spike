// 시험 앱 A (Tauri) — View. 상태·규칙은 Rust(fd-host/fd-viewmodel)에 있고, 여기서는 그리기와 입력 전달만 한다.
// 계약: docs/shell-contract.md. 목록은 가상화한다 — Rust 에서 보이는 범위만 받아 그린다 (10만 항목을 JS 로 옮기지 않는다).
'use strict';

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// VM 기하 상수와 같아야 한다 (crates/fd-viewmodel/src/layout.rs). 열 수·행 높이는 스냅샷으로 받는다.
const ICON_TILE_W = 116;
const ICON_GAP = 8;
const OVERSCAN_ITEMS = 300;    // 보이는 범위 위·아래로 한 번에 더 받아 둘 항목 수
const PREFETCH_MARGIN = 120;   // 받아 둔 범위 끝이 이만큼 가까워지면 미리 받는다 (스크롤 중 빈 줄 방지)
const CACHE_LIMIT = 6000;
const DRAG_THRESHOLD = 5;      // px
const ENTER_GUARD_MS = 150;    // 확정 직후 같은 Enter 가 목록 Activate 로 새지 않게
const IME_LEAK_GUARD_MS = 300; // 편집이 끝난 직후 남은 조합 글자가 목록 type-ahead 로 새지 않게
const PERF_WATCHDOG_MS = 285000;

let cfg = null;
let activePane = 0;
let lastCommitAt = -1e9;
let pressedPane = null;
let tabPress = null;
let dragHitBusy = false;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const nextFrame = () => new Promise((r) => requestAnimationFrame(r));

// ------------------------------------------------------------------ 칸
const panes = [0, 1].map(makePane);

function makePane(i) {
  const el = document.querySelector(`.pane[data-pane="${i}"]`);
  el.appendChild(document.getElementById('pane-tpl').content.cloneNode(true));
  const q = (s) => el.querySelector(s);
  return {
    i, el,
    list: q('.list'), spacer: q('.spacer'), layer: q('.layer'), sink: q('.sink'), addr: q('.addr'),
    tabsBar: q('.tabs'), tabList: q('.tab-list'), header: q('.header'),
    statusText: q('.status-text'), statusSel: q('.status-sel'),
    btnDetails: q('.view-details'), btnIcons: q('.view-icons'),
    snap: null, version: -1, cache: new Map(), prev: new Map(), inflight: null,
    pool: [], poolView: null, renderQueued: false, refreshing: false, refreshAgain: false,
    dropPos: null, renameEl: null, renamePos: null, imeGuardUntil: 0,
    sinkComposing: false, sinkCompStarted: false, addrField: null,
    firstItemPending: false, firstItemWaiter: null, press: null, autoScrolling: false,
    tabsKey: '',
  };
}

function setActive(p) {
  activePane = p.i;
  for (const x of panes) x.el.classList.toggle('active', x === p);
}

function focusSink(p) {
  setActive(p);
  if (document.activeElement !== p.sink) p.sink.focus({ preventScroll: true });
}

function guarded(p) {
  return performance.now() < p.imeGuardUntil;
}

async function dispatch(p, msg) {
  try {
    await invoke('dispatch', { pane: p.i, msg });
  } catch (e) {
    console.error('dispatch', msg, e);
  }
}

function logShortcut(p, key, action) {
  invoke('log_event', { kind: 'shortcut', fields: [key, action, `pane=${p.i}`] }).catch(() => {});
}

// ------------------------------------------------------------------ 스냅샷 → 화면
async function refresh(p) {
  if (p.refreshing) { p.refreshAgain = true; return; }
  p.refreshing = true;
  try {
    do {
      p.refreshAgain = false;
      applySnap(p, await invoke('snapshot', { pane: p.i }));
    } while (p.refreshAgain);
  } catch (e) {
    console.error('snapshot', e);
  } finally {
    p.refreshing = false;
  }
}

function applySnap(p, s) {
  const old = p.snap;
  const layoutChanged = !old || s.folder !== old.folder || s.view !== old.view || s.sort !== old.sort
    || s.descending !== old.descending || s.columns !== old.columns || s.activeTab !== old.activeTab;
  if (s.version !== p.version || layoutChanged) {
    // 같은 폴더·같은 배치면 새 줄이 올 때까지 옛 줄을 보여 준다 (클릭할 때마다 깜빡이지 않게)
    p.prev = layoutChanged ? new Map() : p.cache;
    p.cache = new Map();
    p.version = s.version;
    p.inflight = null;
  }
  p.snap = s;
  if (old && old.folder !== s.folder) p.list.scrollTop = 0;

  // 탭
  const key = JSON.stringify([s.tabs, s.activeTab]);
  if (key !== p.tabsKey) {
    p.tabsKey = key;
    p.tabList.replaceChildren(...s.tabs.map((t, idx) => makeTab(p, t, idx, idx === s.activeTab)));
  }
  if (document.activeElement !== p.addr) p.addr.value = s.folder;
  p.statusText.textContent = s.status;
  p.statusSel.textContent = s.selectedCount > 0 ? `${s.selectedCount}개 선택` : '';
  p.el.classList.toggle('icons', s.view === 'icons');
  p.btnDetails.classList.toggle('checked', s.view === 'details');
  p.btnIcons.classList.toggle('checked', s.view === 'icons');
  for (const c of p.header.querySelectorAll('.col')) {
    const on = c.dataset.sort === s.sort;
    c.classList.toggle('sorted', on);
    c.classList.toggle('desc', on && s.descending);
  }
  p.spacer.style.height = `${s.rows * s.rowHeight + (s.view === 'icons' ? ICON_GAP : 0)}px`;
  if (s.focus != null && (!old || old.focus !== s.focus) && !p.autoScrolling) {
    scrollIntoView(p, s.focus);
  }
  renderList(p);
}

function scrollIntoView(p, pos) {
  const s = p.snap;
  const top = Math.floor(pos / s.columns) * s.rowHeight;
  const bottom = top + s.rowHeight;
  const l = p.list;
  if (top < l.scrollTop) l.scrollTop = top;
  else if (bottom > l.scrollTop + l.clientHeight) l.scrollTop = bottom - l.clientHeight;
}

function visibleRange(p) {
  const s = p.snap;
  if (!s || s.len === 0) return [0, -1];
  const top = p.list.scrollTop;
  const h = p.list.clientHeight;
  const r0 = Math.max(0, Math.floor(top / s.rowHeight));
  const r1 = Math.min(s.rows - 1, Math.floor((top + h) / s.rowHeight));
  return [r0 * s.columns, Math.min(s.len - 1, (r1 + 1) * s.columns - 1)];
}

function scheduleRender(p) {
  if (p.renderQueued) return;
  p.renderQueued = true;
  requestAnimationFrame(() => { p.renderQueued = false; renderList(p); });
}

function makeItem(view) {
  const el = document.createElement('div');
  const img = document.createElement('img');
  img.draggable = false;
  const name = document.createElement('span');
  name.className = 'n';
  const it = { el, img, name, mod: null, size: null, key: '' };
  if (view === 'details') {
    el.className = 'row';
    const c1 = document.createElement('div');
    c1.className = 'col name';
    c1.append(img, name);
    it.mod = document.createElement('div');
    it.mod.className = 'col modified';
    it.size = document.createElement('div');
    it.size.className = 'col size';
    el.append(c1, it.mod, it.size);
  } else {
    el.className = 'tile';
    el.append(img, name);
  }
  return it;
}

function renderList(p) {
  const s = p.snap;
  if (!s) return;
  if (p.poolView !== s.view) {
    for (const it of p.pool) it.el.remove();
    p.pool = [];
    p.poolView = s.view;
  }
  const [first, last] = visibleRange(p);
  ensureRows(p, first, last);
  const n = last >= first ? last - first + 1 : 0;
  while (p.pool.length < n) {
    const it = makeItem(s.view);
    p.layer.appendChild(it.el);
    p.pool.push(it);
  }
  for (let k = 0; k < p.pool.length; k++) {
    const it = p.pool[k];
    if (k >= n) { if (it.el.style.display !== 'none') it.el.style.display = 'none'; continue; }
    const pos = first + k;
    paintItem(p, it, pos, p.cache.get(pos) || p.prev.get(pos));
  }
  placeRenameEditor(p);
  if (p.firstItemPending) afterFirstPaint(p);
}

function paintItem(p, it, pos, row) {
  const s = p.snap;
  const el = it.el;
  if (el.style.display === 'none') el.style.display = '';
  el.dataset.pos = pos;
  if (s.view === 'details') {
    el.style.transform = `translateY(${pos * s.rowHeight}px)`;
  } else {
    const c = pos % s.columns;
    const r = Math.floor(pos / s.columns);
    el.style.transform = `translate(${ICON_GAP + c * (ICON_TILE_W + ICON_GAP)}px, ${r * s.rowHeight + ICON_GAP}px)`;
  }
  const base = s.view === 'details' ? 'row' : 'tile';
  if (!row) {
    if (it.key !== '') {
      it.key = '';
      it.name.textContent = '';
      if (it.mod) { it.mod.textContent = ''; it.size.textContent = ''; }
      it.img.removeAttribute('src');
    }
    el.className = `${base} placeholder`;
    el.dataset.valid = '';
    return;
  }
  el.dataset.valid = '1';
  const key = `${row.name}\u0000${row.modified}\u0000${row.size}\u0000${row.isDir}`;
  if (it.key !== key) {
    it.key = key;
    it.name.textContent = row.name;
    it.img.src = row.isDir ? 'icons/folder.svg' : 'icons/file.svg';
    if (it.mod) { it.mod.textContent = row.modified; it.size.textContent = row.size; }
  }
  let cls = base;
  if (row.selected) cls += ' sel';
  if (row.focused) cls += ' focus';
  if (p.dropPos === pos) cls += ' drop';
  if (el.className !== cls) el.className = cls;
}

function ensureRows(p, first, last) {
  const s = p.snap;
  if (!s || last < first) return;
  const lo = Math.max(0, first - PREFETCH_MARGIN);
  const hi = Math.min(s.len - 1, last + PREFETCH_MARGIN);
  let missing = false;
  for (let k = lo; k <= hi; k++) if (!p.cache.has(k)) { missing = true; break; }
  if (!missing) return;
  if (p.inflight && p.inflight.version === p.version) return; // 한 번에 하나. 응답이 오면 다시 본다
  const start = Math.max(0, first - OVERSCAN_ITEMS);
  const count = Math.min(2000, last - first + 1 + 2 * OVERSCAN_ITEMS);
  const req = { start, version: p.version };
  p.inflight = req;
  invoke('rows', { pane: p.i, start, count })
    .then((rows) => {
      if (p.inflight === req) p.inflight = null;
      if (req.version !== p.version) { scheduleRender(p); return; } // 그사이 내용이 바뀌었다 — 버리고 다시 받는다
      for (const r of rows) p.cache.set(r.pos, r);
      if (p.cache.size > CACHE_LIMIT) trimCache(p);
      if (!p.autoScrolling) renderList(p);
    })
    .catch((e) => { if (p.inflight === req) p.inflight = null; console.error('rows', e); });
}

function trimCache(p) {
  const [first, last] = visibleRange(p);
  const keepLo = first - 2 * OVERSCAN_ITEMS;
  const keepHi = last + 2 * OVERSCAN_ITEMS;
  for (const k of p.cache.keys()) if (k < keepLo || k > keepHi) p.cache.delete(k);
  p.prev = new Map();
}

// FirstItem: 첫 항목이 들어온 뒤 **그려진 프레임** 까지. rAF 두 번 = 다음 프레임이 화면에 나간 뒤.
function afterFirstPaint(p) {
  const [first] = visibleRange(p);
  if (!p.cache.has(first)) return;
  p.firstItemPending = false;
  requestAnimationFrame(() => requestAnimationFrame(() => {
    const t = performance.now();
    const w = p.firstItemWaiter;
    p.firstItemWaiter = null;
    if (w) w(t);
  }));
}

// ------------------------------------------------------------------ 탭
function makeTab(p, t, idx, active) {
  const el = document.createElement('div');
  el.className = `tab${active ? ' active' : ''}`;
  el.title = t.title;
  const tt = document.createElement('span');
  tt.className = 't';
  tt.textContent = t.title;
  const x = document.createElement('span');
  x.className = 'x';
  x.textContent = '×';
  el.append(tt, x);
  el.addEventListener('mousedown', (e) => {
    if (e.button === 1) { e.preventDefault(); invoke('tab_op', { op: 'close', pane: p.i, idx }); return; }
    if (e.button !== 0) return;
    if (e.target === x) { e.preventDefault(); invoke('tab_op', { op: 'close', pane: p.i, idx }); return; }
    e.preventDefault();
    tabPress = { pane: p.i, idx, x: e.clientX, y: e.clientY, dragging: false, el, title: t.title, ghost: null };
  });
  return el;
}

function paneFromPoint(x, y) {
  const el = document.elementFromPoint(x, y);
  const pe = el && el.closest('.pane');
  return pe ? panes[+pe.dataset.pane] : null;
}

function tabInsertIndex(p, x) {
  const tabs = [...p.tabList.children];
  for (let k = 0; k < tabs.length; k++) {
    const r = tabs[k].getBoundingClientRect();
    if (x < r.left + r.width / 2) return k;
  }
  return tabs.length;
}

// ------------------------------------------------------------------ 이름 바꾸기 · 주소줄 (한글 입력 L2)
// Enter 한 번 = 조합 확정 + 동작 정확히 한 번 (엘레나 design §3-5).
// 조합 중 Enter 는 "확정 뒤 실행" 으로 미루고, compositionend 뒤 입력칸의 최종 값으로 한 번만 실행한다.
function imeField(input, { onCommit, onCancel }) {
  const st = { composing: false, pendingEnter: false, done: false, timer: null };
  const commit = (how) => {
    if (st.done) return;
    st.done = true;
    clearTimeout(st.timer);
    lastCommitAt = performance.now();
    onCommit(input.value, how);
  };
  input.addEventListener('compositionstart', () => { st.composing = true; });
  input.addEventListener('compositionend', () => {
    st.composing = false;
    if (st.pendingEnter) {
      st.pendingEnter = false;
      setTimeout(() => commit('enter-after-compositionend'), 0);
    }
  });
  input.addEventListener('keydown', (e) => {
    e.stopPropagation();
    if (e.key === 'Enter' || e.code === 'Enter' || e.code === 'NumpadEnter') {
      e.preventDefault();
      if (e.isComposing || st.composing || e.keyCode === 229) {
        st.pendingEnter = true;
        clearTimeout(st.timer);
        // 확정 이벤트가 끝내 오지 않는 입력기 대비
        st.timer = setTimeout(() => { if (st.pendingEnter) { st.pendingEnter = false; commit('enter-timeout'); } }, 300);
        return;
      }
      commit('enter');
    } else if (e.key === 'Escape' || e.code === 'Escape') {
      e.preventDefault();
      if (st.done) return;
      st.done = true;
      st.pendingEnter = false;
      clearTimeout(st.timer);
      onCancel('esc');
    } else if ((e.ctrlKey || e.metaKey) && e.code === 'KeyA') {
      e.preventDefault();
      input.select();
    }
  });
  return { reset() { st.done = false; st.composing = false; st.pendingEnter = false; clearTimeout(st.timer); } };
}

function placeRenameEditor(p) {
  const s = p.snap;
  if (s.renaming == null) {
    if (p.renameEl) removeRename(p);
    return;
  }
  const pos = s.renaming;
  if (!p.renameEl || p.renamePos !== pos) {
    const row = p.cache.get(pos) || p.prev.get(pos);
    if (!row) return;
    if (p.renameEl) removeRename(p);
    const input = document.createElement('input');
    input.className = 'rename';
    input.spellcheck = false;
    input.value = row.name;
    imeField(input, {
      onCommit: (text, how) => {
        removeRename(p);
        endEdit(p);
        invoke('log_event', { kind: 'shortcut', fields: ['Enter', `rename-commit:${how}`, `pane=${p.i}`] }).catch(() => {});
        dispatch(p, { type: 'commitRename', text });
      },
      onCancel: (how) => cancelRename(p, how),
    });
    p.renameEl = input;
    p.renamePos = pos;
    p.layer.appendChild(input);
    input.focus({ preventScroll: true });
    const dot = row.isDir ? -1 : row.name.lastIndexOf('.');
    input.setSelectionRange(0, dot > 0 ? dot : row.name.length);
  }
  const el = p.renameEl;
  if (s.view === 'details') {
    el.style.left = '30px';
    el.style.top = `${pos * s.rowHeight + 1}px`;
    el.style.width = `${Math.max(120, p.header.querySelector('.col.name').getBoundingClientRect().width - 40)}px`;
  } else {
    const c = pos % s.columns;
    const r = Math.floor(pos / s.columns);
    el.style.left = `${ICON_GAP + c * (ICON_TILE_W + ICON_GAP) - 8}px`;
    el.style.top = `${r * s.rowHeight + ICON_GAP + 86}px`;
    el.style.width = `${ICON_TILE_W + 16}px`;
  }
}

function removeRename(p) {
  if (!p.renameEl) return;
  const el = p.renameEl;
  p.renameEl = null;
  p.renamePos = null;
  el.remove();
}

// 편집이 끝나면 목록으로 돌아가되, 남은 조합 글자가 type-ahead 로 새지 않게 잠깐 막는다 (한-2·한-3)
function endEdit(p) {
  p.imeGuardUntil = performance.now() + IME_LEAK_GUARD_MS;
  p.sink.value = '';
  focusSink(p);
}

function cancelRename(p, how) {
  if (!p.renameEl) return;
  removeRename(p);
  logShortcut(p, how === 'esc' ? 'Esc' : 'click-away', 'rename-cancel');
  dispatch(p, { type: 'cancelRename' });
  endEdit(p);
}

function focusAddr(p) {
  setActive(p);
  p.addr.value = p.snap ? p.snap.folder : p.addr.value;
  p.addrField.reset();
  p.addr.focus({ preventScroll: true });
  p.addr.select();
}

// ------------------------------------------------------------------ 목록 키보드 · type-ahead
function typeAhead(p, text, committed) {
  if (!text || guarded(p)) return;
  invoke('type_ahead', { pane: p.i, text, committed }).catch((e) => console.error(e));
}

function onListKey(p, e) {
  const ctrl = e.ctrlKey || e.metaKey;
  // IME 가 처리 중인 키는 건드리지 않는다. 단 Ctrl 조합은 IME 로 보내지 않는다 (한-5).
  if ((e.isComposing || e.keyCode === 229) && !ctrl) return;
  const s = p.snap;
  if (!s) return;
  const handled = () => { e.preventDefault(); e.stopPropagation(); };
  const code = e.code;
  if (ctrl && code === 'KeyA') { handled(); dispatch(p, { type: 'selectAll' }); logShortcut(p, 'Ctrl+A', 'select-all'); return; }
  if (ctrl && code === 'KeyL') { handled(); focusAddr(p); logShortcut(p, 'Ctrl+L', 'address'); return; }
  if (e.altKey && e.key === 'ArrowUp') { handled(); dispatch(p, { type: 'up' }); logShortcut(p, 'Alt+Up', 'up'); return; }
  if (ctrl || e.altKey) return;
  const cols = s.columns;
  const page = Math.max(1, Math.floor(p.list.clientHeight / s.rowHeight) - 1) * cols;
  const move = (delta) => { handled(); dispatch(p, { type: 'moveFocus', delta, shift: e.shiftKey }); };
  switch (e.key) {
    case 'ArrowDown': return move(cols);
    case 'ArrowUp': return move(-cols);
    case 'ArrowRight': if (cols > 1) move(1); return;
    case 'ArrowLeft': if (cols > 1) move(-1); return;
    case 'PageDown': return move(page);
    case 'PageUp': return move(-page);
    case 'Home': return move(-1e9);
    case 'End': return move(1e9);
    case 'Enter':
      handled();
      if (e.repeat || performance.now() - lastCommitAt < ENTER_GUARD_MS) { logShortcut(p, 'Enter', 'ignored-after-commit'); return; }
      if (s.focus != null) { dispatch(p, { type: 'activate', pos: s.focus }); logShortcut(p, 'Enter', 'activate'); }
      return;
    case 'Backspace': handled(); dispatch(p, { type: 'up' }); logShortcut(p, 'Backspace', 'up'); return;
    case 'F2': handled(); dispatch(p, { type: 'beginRename' }); logShortcut(p, 'F2', 'rename'); return;
    case 'F5': handled(); dispatch(p, { type: 'refresh' }); logShortcut(p, 'F5', 'refresh'); return;
    case 'Escape': handled(); logShortcut(p, 'Esc', 'list'); return;
    case 'Tab': handled(); focusSink(panes[1 - p.i]); return;
    default:
  }
}

function wireSink(p) {
  const sink = p.sink;
  sink.addEventListener('keydown', (e) => onListKey(p, e));
  sink.addEventListener('compositionstart', () => { p.sinkComposing = true; p.sinkCompStarted = !guarded(p); });
  sink.addEventListener('compositionupdate', (e) => {
    if (p.sinkCompStarted && e.data) typeAhead(p, e.data, false);
  });
  sink.addEventListener('compositionend', (e) => {
    const ok = p.sinkCompStarted && !guarded(p);
    p.sinkComposing = false;
    p.sinkCompStarted = false;
    if (ok && e.data) typeAhead(p, e.data, true);
    sink.value = '';
  });
  sink.addEventListener('input', (e) => {
    // 조합 글자는 compositionend 에서 처리했다 — 여기서 또 넣으면 type-ahead 가 두 번 들어간다
    if (e.isComposing || p.sinkComposing || (e.inputType || '').includes('Composition')) return;
    const t = e.data || sink.value;
    sink.value = '';
    if (t) typeAhead(p, t, true);
  });
}

// ------------------------------------------------------------------ 마우스: 선택 · 끌기 시작
function itemPosFromEvent(p, e) {
  const el = e.target.closest('.row, .tile');
  if (!el || el.dataset.valid !== '1') return null;
  return +el.dataset.pos;
}

function wireMouse(p) {
  p.el.addEventListener('mousedown', () => setActive(p));
  p.list.addEventListener('mousedown', (e) => {
    if (e.target === p.renameEl) return;
    if (e.button !== 0) return;
    if (e.target === p.list && e.offsetX >= p.list.clientWidth) return; // 스크롤바
    e.preventDefault(); // 포커스를 목록 입력칸(sink)에 둔다
    focusSink(p);
    const pos = itemPosFromEvent(p, e);
    if (pos == null) {
      if (e.target === p.list || e.target === p.layer || e.target === p.spacer || e.target.closest('.row, .tile')) {
        dispatch(p, { type: 'clickEmpty' });
      }
      return;
    }
    const row = p.cache.get(pos) || p.prev.get(pos);
    const ctrl = e.ctrlKey || e.metaKey;
    const shift = e.shiftKey;
    const selected = !!(row && row.selected);
    // 수정키가 있으면 놓을 때 처리한다 — Shift+끌기(이동)가 범위 선택으로 바뀌면 안 된다
    const deferred = ctrl || shift || selected;
    p.press = { x: e.clientX, y: e.clientY, pos, ctrl, shift, deferred, selected, dragging: false };
    pressedPane = p;
    if (!deferred) dispatch(p, { type: 'click', pos, ctrl: false, shift: false });
  });
  p.list.addEventListener('dblclick', (e) => {
    const pos = itemPosFromEvent(p, e);
    if (pos != null) dispatch(p, { type: 'activate', pos });
  });
  p.list.addEventListener('scroll', () => { if (!p.autoScrolling) scheduleRender(p); });
  new ResizeObserver(() => {
    dispatch(p, { type: 'setViewport', width: p.list.clientWidth, height: p.list.clientHeight }).then(() => refresh(p));
  }).observe(p.list);
}

async function startDragOut(p, e) {
  const pr = p.press;
  pr.dragging = true;
  const shift = e.shiftKey || pr.shift;
  if (!pr.selected) await dispatch(p, { type: 'click', pos: pr.pos, ctrl: false, shift: false });
  try {
    await invoke('drag_out', { pane: p.i, shift });
  } catch (err) {
    invoke('log_event', { kind: 'drag-out-error', fields: [String(err)] }).catch(() => {});
  }
}

window.addEventListener('mousemove', (e) => {
  if (tabPress) { onTabDragMove(e); return; }
  const p = pressedPane;
  if (!p || !p.press || p.press.dragging) return;
  if (!(e.buttons & 1)) { p.press = null; pressedPane = null; return; }
  if (Math.hypot(e.clientX - p.press.x, e.clientY - p.press.y) < DRAG_THRESHOLD) return;
  startDragOut(p, e);
});

window.addEventListener('mouseup', (e) => {
  if (tabPress) { onTabDragEnd(e); return; }
  const p = pressedPane;
  pressedPane = null;
  if (!p || !p.press) return;
  const pr = p.press;
  p.press = null;
  if (!pr.dragging && pr.deferred) dispatch(p, { type: 'click', pos: pr.pos, ctrl: pr.ctrl, shift: pr.shift });
});

// 이름 바꾸기 중 다른 곳을 누르면 취소 (엘레나 한-3 기준)
document.addEventListener('mousedown', (e) => {
  for (const p of panes) if (p.renameEl && e.target !== p.renameEl) cancelRename(p, 'click-away');
}, true);

function onTabDragMove(e) {
  const t = tabPress;
  if (!t.dragging) {
    if (Math.hypot(e.clientX - t.x, e.clientY - t.y) < 6) return;
    t.dragging = true;
    t.el.classList.add('dragging');
    t.ghost = document.createElement('div');
    t.ghost.className = 'drag-ghost';
    t.ghost.textContent = t.title;
    document.body.appendChild(t.ghost);
  }
  t.ghost.style.left = `${e.clientX + 12}px`;
  t.ghost.style.top = `${e.clientY + 8}px`;
  const target = paneFromPoint(e.clientX, e.clientY);
  for (const p of panes) p.tabsBar.classList.toggle('tab-drop', !!target && target === p && p.i !== t.pane);
}

function onTabDragEnd(e) {
  const t = tabPress;
  tabPress = null;
  for (const p of panes) p.tabsBar.classList.remove('tab-drop');
  if (t.ghost) t.ghost.remove();
  t.el.classList.remove('dragging');
  if (!t.dragging) { invoke('tab_op', { op: 'activate', pane: t.pane, idx: t.idx }); return; }
  const target = paneFromPoint(e.clientX, e.clientY);
  if (target && target.i !== t.pane) {
    invoke('tab_op', { op: 'move', pane: t.pane, idx: t.idx, to: target.i, insertAt: tabInsertIndex(target, e.clientX) });
  }
}

// ------------------------------------------------------------------ OS 끌어다 놓기 (L1)
function contentPoint(p, x, y) {
  const r = p.list.getBoundingClientRect();
  if (y < r.top) return { x: -1, y: -1 }; // 탭·도구 줄 위 = 빈 곳 (현재 폴더)
  return { x: x - r.left, y: y - r.top + p.list.scrollTop - (p.snap && p.snap.view === 'icons' ? ICON_GAP : 0) };
}

function setDropPos(target, pos) {
  for (const p of panes) {
    const v = p === target ? pos : null;
    if (p.dropPos !== v) { p.dropPos = v; scheduleRender(p); }
  }
}

async function onOsDrag(d) {
  if (d.phase === 'leave') { setDropPos(null, null); return; }
  const p = paneFromPoint(d.x, d.y);
  if (d.phase === 'drop') {
    setDropPos(null, null);
    if (!p) { invoke('log_event', { kind: 'drop-outside', fields: [String(d.x), String(d.y)] }).catch(() => {}); return; }
    const c = contentPoint(p, d.x, d.y);
    await invoke('drop_at', { pane: p.i, x: c.x, y: c.y, paths: d.paths, mode: d.mode, origin: d.origin });
    return;
  }
  // enter / over: 놓기 직전 대상 폴더 줄 강조 (끌-1). 판정은 VM 기하 (hit).
  if (!p) { setDropPos(null, null); return; }
  if (dragHitBusy) return;
  dragHitBusy = true;
  try {
    const c = contentPoint(p, d.x, d.y);
    const pos = await invoke('hit', { pane: p.i, x: c.x, y: c.y });
    const row = pos != null ? p.cache.get(pos) : null;
    setDropPos(p, row && row.isDir ? pos : null);
  } finally {
    dragHitBusy = false;
  }
}

// ------------------------------------------------------------------ 자동 측정 (FD_SPIKE_PERF_AUTO=1)
async function waitLoaded(p, timeoutMs = 60000) {
  const t0 = performance.now();
  for (;;) {
    const s = await invoke('snapshot', { pane: p.i });
    if (!s.loading) { applySnap(p, s); return s; }
    if (performance.now() - t0 > timeoutMs) throw new Error('폴더 읽기가 끝나지 않음');
    await sleep(30);
  }
}

function withTimeout(promise, ms, what) {
  return Promise.race([promise, sleep(ms).then(() => { throw new Error(`${what} 시간 초과`); })]);
}

function autoScroll(p, seconds, pxPerS) {
  return new Promise((resolve) => {
    const frames = [];
    let start = null;
    let last = null;
    p.autoScrolling = true;
    const tick = (ts) => {
      if (start === null) { start = ts; last = ts; } else { frames.push(ts - last); last = ts; }
      const l = p.list;
      l.scrollTop = Math.min(l.scrollHeight - l.clientHeight, ((ts - start) / 1000) * pxPerS);
      renderList(p); // 같은 프레임 안에서 그린다 — scroll 이벤트를 기다리면 한 프레임 늦다
      if (ts - start < seconds * 1000) requestAnimationFrame(tick);
      else { p.autoScrolling = false; resolve(frames); }
    };
    requestAnimationFrame(tick);
  });
}

async function perfAuto() {
  const p = panes[0];
  const target = cfg.perfTarget;
  setTimeout(() => {
    invoke('log_event', { kind: 'perf-error', fields: ['watchdog'] }).finally(() => invoke('exit_app', { code: 1 }));
  }, PERF_WATCHDOG_MS);
  try {
    await invoke('log_event', { kind: 'perf-method', fields: ['raf-timestamp', 'scroll=rAF·scrollTop', `dpr=${devicePixelRatio}`] });
    await waitLoaded(p);
    for (let k = 0; k < cfg.openRepeats; k++) {
      await dispatch(p, { type: 'up' });
      await waitLoaded(p);
      const painted = new Promise((r) => { p.firstItemWaiter = r; });
      const t0 = performance.now();
      await dispatch(p, { type: 'navigate', text: target });
      const t1 = await withTimeout(painted, 30000, 'FirstItem');
      await invoke('perf', { point: 'FirstItem', ms: t1 - t0 });
      await waitLoaded(p);
    }
    for (const [view, point] of [['details', 'Frame:details'], ['icons', 'Frame:icons']]) {
      await dispatch(p, { type: 'setView', view });
      applySnap(p, await invoke('snapshot', { pane: p.i }));
      p.list.scrollTop = 0;
      renderList(p);
      await sleep(cfg.settleMs);
      const frames = await autoScroll(p, cfg.scrollSeconds, cfg.scrollPxPerS);
      await invoke('perf_frames', { point, samples: frames });
    }
    await invoke('exit_app', { code: 0 });
  } catch (err) {
    await invoke('log_event', { kind: 'perf-error', fields: [String(err)] }).catch(() => {});
    await invoke('exit_app', { code: 1 });
  }
}

// ------------------------------------------------------------------ 시작
async function main() {
  for (const p of panes) {
    wireSink(p);
    wireMouse(p);
    p.addrField = imeField(p.addr, {
      onCommit: (text) => {
        logShortcut(p, 'Enter', 'address-navigate');
        dispatch(p, { type: 'navigate', text });
        endEdit(p);
      },
      onCancel: () => {
        logShortcut(p, 'Esc', 'address-cancel');
        p.addr.value = p.snap ? p.snap.folder : '';
        endEdit(p);
      },
    });
    p.addr.addEventListener('focus', () => { setActive(p); p.addrField.reset(); });
    p.addr.addEventListener('blur', () => { if (p.snap) p.addr.value = p.snap.folder; });
    p.el.querySelector('.up').addEventListener('click', () => { dispatch(p, { type: 'up' }); focusSink(p); });
    p.el.querySelector('.refresh').addEventListener('click', () => { dispatch(p, { type: 'refresh' }); focusSink(p); });
    p.btnDetails.addEventListener('click', () => { dispatch(p, { type: 'setView', view: 'details' }).then(() => refresh(p)); focusSink(p); });
    p.btnIcons.addEventListener('click', () => { dispatch(p, { type: 'setView', view: 'icons' }).then(() => refresh(p)); focusSink(p); });
    p.el.querySelector('.tab-new').addEventListener('click', () => invoke('tab_op', { op: 'new', pane: p.i, idx: 0 }));
    for (const c of p.header.querySelectorAll('.col')) {
      c.addEventListener('click', () => { dispatch(p, { type: 'setSort', key: c.dataset.sort }); focusSink(p); });
    }
  }
  await listen('fd', (ev) => {
    const { kind, pane } = ev.payload;
    if (kind === 'tabs') { refresh(panes[0]); refresh(panes[1]); return; }
    const p = panes[pane];
    if (!p) return;
    if (kind === 'first-item') p.firstItemPending = true;
    refresh(p);
  });
  await listen('os-drag', (ev) => { onOsDrag(ev.payload).catch((e) => console.error(e)); });

  // ColdStart = 프로세스 시작 → 왼쪽 칸 첫 항목이 그려진 프레임
  panes[0].firstItemWaiter = () => invoke('cold_start');
  for (const p of panes) {
    await dispatch(p, { type: 'setViewport', width: p.list.clientWidth, height: p.list.clientHeight });
  }
  cfg = await invoke('ui_ready');
  document.title = cfg.title;
  focusSink(panes[0]);
  await Promise.all(panes.map(refresh));
  if (cfg.perfAuto) perfAuto();
}

main().catch((e) => {
  console.error(e);
  invoke('log_event', { kind: 'ui-error', fields: [String(e)] }).catch(() => {});
});
