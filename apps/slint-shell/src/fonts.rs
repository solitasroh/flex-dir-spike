//! 글꼴 순서 Roboto → Pretendard → 시스템 기본 (계약 §3, 엘레나 A-2).
//!
//! - 두 글꼴은 `ui/app.slint` 의 `import "…ttf/otf";` 로 바이너리에 **원본 그대로** 들어간다.
//!   창의 `default-font-family: "Roboto"`.
//! - Slint 의 `font-family` 는 한 이름만 받는다. Roboto 에 없는 글자는 parley 가 **문자 체계별 폴백 목록**
//!   (fontique `Collection`)에서 찾는다. 이 목록은 기본이 시스템(fontconfig) 이 고른 한 글꼴이라 한글이
//!   Noto Sans CJK 등으로 갈 수 있다 → 한글(Hang)·가나(Hira/Kana)의 폴백 맨 앞에 Pretendard 를 끼운다.
//!   그 뒤는 시스템 폴백 그대로(한자·이모지·아랍어는 시스템).
//! - Slint 1.19 는 **Qt 백엔드도** parley+fontique 로 글자를 배치하고 QRawFont 로 그리므로
//!   (i-slint-backend-qt `qt_window.rs` FontCache) winit·Qt 두 백엔드에 이 한 가지 방법이 같이 먹는다.
//!   fontconfig 설정 파일(FONTCONFIG_FILE)은 쓰지 않는다 — Slint 가 Qt 의 글꼴 폴백을 거치지 않기 때문.

use slint::fontique_012::fontique;

pub const PRIMARY: &str = "Roboto";
pub const HANGUL: &str = "Pretendard";

/// 플랫폼(백엔드)이 선 뒤에 부른다. 결과 문구는 events.log `fonts` 에 남긴다.
pub fn install_fallbacks() -> Result<String, String> {
    let mut coll = slint::fontique_012::shared_collection();
    if coll.family_id(PRIMARY).is_none() {
        return Err(format!("{PRIMARY} 가 글꼴 모음에 없다 (import 확인)"));
    }
    let Some(pre) = coll.family_id(HANGUL) else {
        return Err(format!("{HANGUL} 가 글꼴 모음에 없다 (import 확인)"));
    };
    let mut done = Vec::new();
    for script in [*b"Hang", *b"Hira", *b"Kana"] {
        let key = fontique::FallbackKey::new(fontique::Script::from_bytes(script), None);
        let mut families = vec![pre];
        families.extend(coll.fallback_families(key).filter(|f| *f != pre));
        let n = families.len();
        if coll.set_fallbacks(key, families.into_iter()) {
            done.push(format!("{}={HANGUL}+{}", String::from_utf8_lossy(&script), n - 1));
        }
    }
    Ok(done.join(","))
}
