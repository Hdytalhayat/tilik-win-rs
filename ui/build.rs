fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/tilik.ico");
        res.set("FileDescription", "Tilik - fast local file search");
        res.set("ProductName", "Tilik");
        res.set("ProductVersion", env!("CARGO_PKG_VERSION"));
        res.set("LegalCopyright", "Copyright (c) 2026");
        res.compile().expect("failed to compile Windows resources");
    }
}