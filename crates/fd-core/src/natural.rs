//! 자연 정렬 (`file2 < file10`). 입력은 `name::sort_key` 를 거친 키다.
use std::cmp::Ordering;

pub fn compare(a: &str, b: &str) -> Ordering {
    let (mut ia, mut ib) = (a.char_indices().peekable(), b.char_indices().peekable());
    loop {
        match (ia.peek().copied(), ib.peek().copied()) {
            (None, None) => return a.len().cmp(&b.len()),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some((sa, ca)), Some((sb, cb))) => {
                if ca.is_ascii_digit() && cb.is_ascii_digit() {
                    let ea = digits_end(a, sa);
                    let eb = digits_end(b, sb);
                    let (na, nb) = (a[sa..ea].trim_start_matches('0'), b[sb..eb].trim_start_matches('0'));
                    let ord = na.len().cmp(&nb.len()).then_with(|| na.cmp(nb));
                    if ord != Ordering::Equal {
                        return ord;
                    }
                    // 같은 값이면 앞 0 이 적은 쪽이 먼저 (a1 < a01)
                    let z = (ea - sa).cmp(&(eb - sb));
                    if z != Ordering::Equal {
                        return z;
                    }
                    while ia.peek().is_some_and(|&(i, _)| i < ea) {
                        ia.next();
                    }
                    while ib.peek().is_some_and(|&(i, _)| i < eb) {
                        ib.next();
                    }
                } else {
                    let ord = ca.cmp(&cb);
                    if ord != Ordering::Equal {
                        return ord;
                    }
                    ia.next();
                    ib.next();
                }
            }
        }
    }
}

fn digits_end(s: &str, start: usize) -> usize {
    s[start..].find(|c: char| !c.is_ascii_digit()).map_or(s.len(), |i| start + i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::name::sort_key;
    use unicode_normalization::UnicodeNormalization;

    #[test]
    fn numbers_compare_by_value() {
        let mut v = vec!["file10", "file2", "file1", "file02", "a", "file"];
        v.sort_by(|a, b| compare(a, b));
        assert_eq!(v, ["a", "file", "file1", "file2", "file02", "file10"]);
    }

    #[test]
    fn nfd_hangul_sorts_with_nfc() {
        // 브라이언 결함 재현 3: NFD 로 저장된 한글이 "가나" 보다 앞으로 정렬됐다.
        let nfd_da: String = "다라".nfd().collect();
        let mut v = vec![sort_key("나"), sort_key(&nfd_da), sort_key("가나"), sort_key("abc")];
        v.sort_by(|a, b| compare(a, b));
        assert_eq!(v, ["abc", "가나", "나", "다라"]);
    }
}
