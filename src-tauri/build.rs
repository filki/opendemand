fn main() {
    println!("cargo:rerun-if-changed=../docs/architecture/schema");
    #[cfg(feature = "desktop")]
    tauri_build::build();
}
