//! 이름 비교 정책. **디스크마다** 다르다 — 같은 맥에서도 APFS 볼륨은 대소문자 무시,
//! 외장 ext4 는 구분이다. 그래서 OS 상수가 아니라 값으로 받는다 (브라이언 qa §4).
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NamePolicy {
    pub case_sensitive: bool,
}

impl NamePolicy {
    pub const LINUX_DEFAULT: Self = Self { case_sensitive: true };
    pub const WINDOWS_DEFAULT: Self = Self { case_sensitive: false };
    pub const MAC_DEFAULT: Self = Self { case_sensitive: false };

    pub const fn native() -> Self {
        if cfg!(target_os = "linux") { Self::LINUX_DEFAULT } else { Self::WINDOWS_DEFAULT }
    }

    /// 동일성 키. 이 키가 같으면 같은 항목이다. NFC 는 항상, 대소문자는 정책대로.
    pub fn identity_key(&self, name: &str) -> String {
        let nfc: String = name.nfc().collect();
        if self.case_sensitive { nfc } else { nfc.to_lowercase() }
    }
}

/// 정렬·type-ahead 용 키. 표시 순서는 디스크 정책과 무관하게 대소문자를 무시한다
/// (탐색기·Finder·Files 모두 그렇다). NFC 로 맞춘다 — NFD 한글이 앞에 몰리지 않게.
pub fn sort_key(name: &str) -> String {
    name.nfc().collect::<String>().to_lowercase()
}

/// IME 조합 중 미리 찾기용 접두 비교. 한글을 자모로 풀어(NFKD) 비교한다 —
/// 조합 중인 `하` 가 `한글` 의 앞부분으로, 낱자 `ㅎ` 이 `하늘` 의 앞부분으로 맞는다.
/// `sort_key` 는 `sort_key()` 를 거친 값(소문자·NFC)이다. 대부분 첫 글자에서 끝나므로 10만 항목에서도 싸다.
pub fn jamo_prefix(sort_key: &str, needle: &str) -> bool {
    let mut hay = sort_key.nfkd();
    needle.nfkd().flat_map(char::to_lowercase).all(|n| hay.next() == Some(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_sensitive_disk_keeps_both_names() {
        // 브라이언 결함 재현 2: a.txt 와 A.txt 가 한 줄로 합쳐졌다.
        let p = NamePolicy::LINUX_DEFAULT;
        assert_ne!(p.identity_key("a.txt"), p.identity_key("A.txt"));
        let w = NamePolicy::WINDOWS_DEFAULT;
        assert_eq!(w.identity_key("a.txt"), w.identity_key("A.txt"));
    }

    #[test]
    fn composing_jamo_matches_syllable_prefix() {
        assert!(jamo_prefix(&sort_key("한글.txt"), "하"));
        assert!(jamo_prefix(&sort_key("한글.txt"), "한"));
        assert!(jamo_prefix(&sort_key("한글.txt"), "\u{314E}"), "낱자 ㅎ");
        assert!(jamo_prefix(&sort_key("한글.txt"), "한\u{3131}"), "확정 한 + 조합 ㄱ");
        assert!(!jamo_prefix(&sort_key("ghost.txt"), "한"));
        assert!(!jamo_prefix(&sort_key("하늘.txt"), "한"));
        assert!(jamo_prefix(&sort_key("Apple.txt"), "AP"));
    }

    #[test]
    fn nfd_hangul_is_same_name_as_nfc() {
        // 브라이언 결함 재현 4: 맥에서 만든 NFD 이름을 다른 폴더/항목으로 판정했다.
        let nfc = "가나.txt";
        let nfd: String = nfc.nfd().collect();
        assert_ne!(nfc, nfd);
        for p in [NamePolicy::LINUX_DEFAULT, NamePolicy::MAC_DEFAULT] {
            assert_eq!(p.identity_key(nfc), p.identity_key(&nfd));
        }
        assert_eq!(sort_key(nfc), sort_key(&nfd));
    }
}
