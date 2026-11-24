use super::*;
use unicode_normalization_shim::nfd;

mod unicode_normalization_shim {
    // fd-core 가 이미 unicode-normalization 을 쓴다. 테스트에서만 NFD 문자열을 만든다.
    pub fn nfd(s: &str) -> String {
        // 한글 음절 → 자모 분해 (유니코드 표준 산식). 외부 의존 없이 테스트 입력만 만든다.
        let mut out = String::new();
        for c in s.chars() {
            let code = c as u32;
            if (0xAC00..=0xD7A3).contains(&code) {
                let s_index = code - 0xAC00;
                let l = 0x1100 + s_index / (21 * 28);
                let v = 0x1161 + (s_index % (21 * 28)) / 28;
                let t = 0x11A7 + s_index % 28;
                out.push(char::from_u32(l).unwrap());
                out.push(char::from_u32(v).unwrap());
                if t != 0x11A7 {
                    out.push(char::from_u32(t).unwrap());
                }
            } else {
                out.push(c);
            }
        }
        out
    }
}

fn vm() -> PaneVm {
    PaneVm::new(PathFlavor::Posix, NamePolicy::LINUX_DEFAULT)
}

fn file(name: &str) -> FileItem {
    FileItem::new(PathBuf::from(format!("/t/{name}")), false, name.len() as u64, None)
}

fn dir(name: &str) -> FileItem {
    FileItem::new(PathBuf::from(format!("/t/{name}")), true, 0, None)
}

fn opened(v: &mut PaneVm) -> RequestId {
    match v.update(PaneMsg::Navigate("/t".into())).as_slice() {
        [Effect::Enumerate { req, folder }] => {
            assert_eq!(folder, "/t");
            *req
        }
        other => panic!("{other:?}"),
    }
}

fn names(v: &PaneVm) -> Vec<String> {
    (0..v.len()).map(|p| v.item(p).unwrap().name.clone()).collect()
}

#[test]
fn first_batch_signals_first_item_once_and_late_batches_of_old_request_are_dropped() {
    let mut v = vm();
    let r1 = opened(&mut v);
    let fx = v.update(PaneMsg::Navigate("/u".into()));
    let r2 = match fx.as_slice() {
        [Effect::Cancel(old), Effect::Enumerate { req, .. }] => {
            assert_eq!(*old, r1);
            *req
        }
        other => panic!("{other:?}"),
    };
    assert!(v.update(PaneMsg::Batch(r1, vec![file("late")])).is_empty());
    assert!(v.is_empty(), "취소된 요청의 배치가 섞였다");
    assert_eq!(v.update(PaneMsg::Batch(r2, vec![file("a")])), vec![Effect::FirstItemReady]);
    assert!(v.update(PaneMsg::Batch(r2, vec![file("b")])).is_empty());
}

#[test]
fn batches_merge_into_natural_order_with_folders_first() {
    let mut v = vm();
    let r = opened(&mut v);
    v.update(PaneMsg::Batch(r, vec![file("file10"), dir("zeta"), file("file2")]));
    v.update(PaneMsg::Batch(r, vec![file("file1"), dir("alpha"), file("B.txt"), file("a.txt")]));
    assert_eq!(names(&v), ["alpha", "zeta", "a.txt", "B.txt", "file1", "file2", "file10"]);
    v.update(PaneMsg::SetSort(SortKey::Name));
    assert_eq!(names(&v), ["zeta", "alpha", "file10", "file2", "file1", "B.txt", "a.txt"], "내림차순도 폴더 먼저");
}

#[test]
fn case_variants_are_two_items_on_case_sensitive_disk() {
    let mut v = vm();
    let r = opened(&mut v);
    v.update(PaneMsg::Batch(r, vec![file("a.txt"), file("A.txt")]));
    assert_eq!(v.len(), 2, "브라이언 결함 재현 2 — 대소문자만 다른 두 파일이 합쳐지면 안 된다");
}

#[test]
fn nfd_hangul_sorts_and_typeaheads_like_nfc() {
    let mut v = vm();
    let r = opened(&mut v);
    v.update(PaneMsg::Batch(r, vec![file(&nfd("다라.txt")), file("나.txt"), file("가나.txt")]));
    assert_eq!(v.item(2).unwrap().name, nfd("다라.txt"));
    v.update(PaneMsg::TypeAhead { text: "다".into(), now_ms: 10_000 });
    assert_eq!(v.focus(), Some(2), "NFC 로 친 '다' 가 NFD 이름을 찾는다");
}

#[test]
fn shift_and_ctrl_click_select_like_explorer() {
    let mut v = vm();
    let r = opened(&mut v);
    v.update(PaneMsg::Batch(r, (0..6).map(|i| file(&format!("f{i}"))).collect()));
    v.update(PaneMsg::Click { pos: 1, ctrl: false, shift: false });
    v.update(PaneMsg::Click { pos: 3, ctrl: false, shift: true });
    assert_eq!((0..6).filter(|&p| v.is_selected(p)).collect::<Vec<_>>(), [1, 2, 3]);
    v.update(PaneMsg::Click { pos: 5, ctrl: true, shift: false });
    v.update(PaneMsg::Click { pos: 2, ctrl: true, shift: false });
    assert_eq!((0..6).filter(|&p| v.is_selected(p)).collect::<Vec<_>>(), [1, 3, 5]);
    assert_eq!(v.selected_count(), 3);
    v.update(PaneMsg::MoveFocus { delta: -1, shift: true });
    assert_eq!(v.focus(), Some(1));
}

#[test]
fn typeahead_accumulates_then_resets_after_a_second() {
    let mut v = vm();
    let r = opened(&mut v);
    v.update(PaneMsg::Batch(r, vec![file("abc"), file("abd"), file("bcd"), file("bce")]));
    v.update(PaneMsg::TypeAhead { text: "b".into(), now_ms: 5000 });
    assert_eq!(v.focus(), Some(2));
    v.update(PaneMsg::TypeAhead { text: "ce".into(), now_ms: 5300 });
    assert_eq!(v.focus(), Some(3));
    v.update(PaneMsg::TypeAhead { text: "a".into(), now_ms: 7000 });
    assert_eq!(v.focus(), Some(0), "1초 뒤에는 새로 시작한다");
}

#[test]
fn refresh_keeps_selection_and_focus_by_name() {
    let mut v = vm();
    let r = opened(&mut v);
    v.update(PaneMsg::Batch(r, vec![file("a"), file("b"), file("c")]));
    v.update(PaneMsg::Click { pos: 1, ctrl: false, shift: false });
    let r2 = match v.update(PaneMsg::Refresh).as_slice() {
        [Effect::Cancel(_), Effect::Enumerate { req, .. }] | [Effect::Enumerate { req, .. }] => *req,
        other => panic!("{other:?}"),
    };
    let fx = v.update(PaneMsg::Batch(r2, vec![file("c"), file("b"), file("new"), file("a")]));
    assert!(fx.is_empty(), "새로 고침은 FirstItem 을 재지 않는다 — 전환의 체감이지 갱신의 체감이 아니다");
    assert!(v.is_selected(1) && v.selected_count() == 1, "갱신 중에도 선택 유지");
    assert_eq!(v.focus(), Some(1));
}

#[test]
fn rename_commits_only_valid_new_names() {
    let mut v = vm();
    let r = opened(&mut v);
    v.update(PaneMsg::Batch(r, vec![file("a.txt")]));
    v.update(PaneMsg::Click { pos: 0, ctrl: false, shift: false });
    v.update(PaneMsg::BeginRename);
    assert_eq!(v.renaming(), Some(0));
    assert!(v.update(PaneMsg::CommitRename("a/b".into())).is_empty());
    v.update(PaneMsg::BeginRename);
    assert_eq!(
        v.update(PaneMsg::CommitRename(" 한글.txt ".into())),
        vec![Effect::Rename { from: "/t/a.txt".into(), to: "/t/한글.txt".into() }]
    );
    assert_eq!(v.renaming(), None);
}

#[test]
fn drop_on_folder_row_targets_that_folder_and_empty_space_targets_current() {
    let mut v = vm();
    let r = opened(&mut v);
    v.update(PaneMsg::Batch(r, vec![dir("sub"), file("x")]));
    let fx = v.update(PaneMsg::Drop { paths: vec!["/home/a.png".into()], target: Some(0), mode: Transfer::Copy });
    assert_eq!(fx, vec![Effect::Transfer { sources: vec!["/home/a.png".into()], dest: "/t/sub".into(), mode: Transfer::Copy }]);
    let fx = v.update(PaneMsg::Drop { paths: vec!["/home/a.png".into()], target: Some(1), mode: Transfer::Move });
    assert_eq!(fx, vec![Effect::Transfer { sources: vec!["/home/a.png".into()], dest: "/t".into(), mode: Transfer::Move }], "파일 위는 현재 폴더");
    assert!(v.update(PaneMsg::Drop { paths: vec!["/t/x".into()], target: None, mode: Transfer::Copy }).is_empty(), "제자리 복사는 하지 않는다");
    assert!(v.update(PaneMsg::Drop { paths: vec!["/t/sub".into()], target: Some(0), mode: Transfer::Move }).is_empty(), "자기 자신 속으로 넣지 않는다");
}

#[test]
fn up_goes_to_parent_and_focuses_the_folder_we_came_from() {
    let mut v = vm();
    let fx = v.update(PaneMsg::Navigate("/t/sub".into()));
    let _ = fx;
    let r = match v.update(PaneMsg::Up).as_slice() {
        [Effect::Cancel(_), Effect::Enumerate { req, folder }] => {
            assert_eq!(folder, "/t");
            *req
        }
        other => panic!("{other:?}"),
    };
    v.update(PaneMsg::Batch(r, vec![dir("a"), dir("sub"), dir("z")]));
    assert_eq!(v.focus(), Some(1));
}

#[test]
fn composing_preview_moves_without_committing_to_buffer() {
    let mut v = vm();
    let r = opened(&mut v);
    v.update(PaneMsg::Batch(r, vec![file("ghost.txt"), file("한글.txt"), file("가방.txt")]));
    // 'ㅎ' → '하' → '한' 조합 중
    v.update(PaneMsg::TypeAheadPreview { composing: "하".into(), now_ms: 1000 });
    assert_eq!(v.item(v.focus().unwrap()).unwrap().name, "한글.txt", "조합 중에도 움직인다");
    // 확정되면 버퍼에 들어가고, 이어지는 조합은 버퍼 + 조합으로 찾는다
    v.update(PaneMsg::TypeAhead { text: "한".into(), now_ms: 1100 });
    v.update(PaneMsg::TypeAheadPreview { composing: "글".into(), now_ms: 1200 });
    assert_eq!(v.item(v.focus().unwrap()).unwrap().name, "한글.txt");
    assert_ne!(v.item(v.focus().unwrap()).unwrap().name, "ghost.txt");
}
