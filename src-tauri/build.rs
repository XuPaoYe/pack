fn main() {
    // is_public_build() 用 option_env! 在编译期取值；env 改了 cargo 默认不会重编。
    // 这行强制让 cargo 在 VITE_SUPERAI_PUBLIC_BUILD 变化时 invalidate 缓存。
    println!("cargo:rerun-if-env-changed=VITE_SUPERAI_PUBLIC_BUILD");
    tauri_build::build()
}
