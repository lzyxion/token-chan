fn main() {
    // 아이콘 파일을 감시 목록에 넣는다 — 없으면 그림만 갈아 끼웠을 때 이 스크립트가
    // 다시 돌지 않아 **exe 에 박힌 옛 아이콘 리소스가 그대로 남는다.** tauri-build 가
    // 스스로 거는 감시는 tauri.conf.json 과 capabilities 뿐이라, icons/ 만 바꾸면
    // 빌드는 성공하는데 아이콘만 옛것인 채로 배포된다 (실제로 한 번 그렇게 나갔다).
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
