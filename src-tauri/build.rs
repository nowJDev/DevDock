// 데스크톱 배포 빌드의 네이티브 자산을 준비합니다.
fn main() {
    #[cfg(feature = "desktop")]
    tauri_build::build()
}
