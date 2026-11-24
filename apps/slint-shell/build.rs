fn main() {
    // 표준 위젯(ListView 스크롤바)은 두 백엔드에서 같은 모양이 되도록 fluent 로 고정한다.
    // (qt 기능으로 빌드하면 기본 스타일이 Qt 네이티브가 된다 — 비교 화면이 백엔드마다 달라지지 않게.)
    let cfg = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", cfg).expect("slint 컴파일");
}
