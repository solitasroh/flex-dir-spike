//! 공용 자산(글꼴·아이콘)을 프론트엔드 폴더로 복사한 뒤 tauri-build 를 부른다.
//! 원본은 저장소 `assets/` 하나뿐이다 — 두 시험 앱이 같은 파일을 쓰게 (엘레나 A-2).
use std::fs;
use std::path::Path;

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("자산 폴더를 만들 수 없음");
    for e in fs::read_dir(from).expect("assets 폴더가 없음") {
        let e = e.expect("assets 읽기 실패");
        let dst = to.join(e.file_name());
        // 내용이 같으면 건드리지 않는다 (불필요한 재빌드 방지)
        let same = fs::read(e.path()).ok() == fs::read(&dst).ok();
        if !same {
            fs::copy(e.path(), &dst).expect("자산 복사 실패");
        }
    }
    println!("cargo:rerun-if-changed={}", from.display());
}

fn main() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo = manifest.join("../../..");
    let ui = manifest.join("../ui");
    copy_dir(&repo.join("assets/fonts"), &ui.join("fonts"));
    copy_dir(&repo.join("assets/icons"), &ui.join("icons"));
    tauri_build::build()
}
