// EXE にアイコンとバージョン情報を埋め込む（エクスプローラーやタスクバーで C# 版と同じアイコンになる）
fn main() {
    println!("cargo:rerun-if-changed=assets/app.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/app.ico");
        res.set("ProductName", "お気に入りショートカット");
        res.set("FileDescription", "お気に入りショートカット（Rust 版）");
        res.set("CompanyName", "YamaPiFIT");
        res.set("LegalCopyright", "Copyright (c) 2026 YamaPiFIT - MIT License");
        if let Err(e) = res.compile() {
            // リソースを付けられなくてもアプリ自体は動くので、警告だけにする
            println!("cargo:warning=アイコンを埋め込めませんでした: {e}");
        }
    }
}
