fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "NeuralSwap");
        res.set("FileDescription", "NeuralSwap — native DLSS 5 neural rendering installer");
        res.set("CompanyName", "craictechnology");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon resource not embedded: {e}");
        }
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
