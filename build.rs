fn main() {
    // Windows 向けビルドのときだけ、EXE にアイコンを埋め込む。
    // `assets/icon.ico` があれば使い、無ければ警告を出して通常ビルドを続行する。
    #[cfg(target_os = "windows")]
    {
        println!("cargo:rerun-if-changed=assets/icon.ico");
        let icon_path = std::path::Path::new("assets/icon.ico");
        if icon_path.exists() {
            let mut res = winresource::WindowsResource::new();
            res.set_icon("assets/icon.ico");
            if let Err(e) = res.compile() {
                println!("cargo:warning=failed to embed EXE icon: {e}");
            }
        } else {
            println!(
                "cargo:warning=assets/icon.ico not found; EXE will be built without a custom icon"
            );
        }
    }
}
