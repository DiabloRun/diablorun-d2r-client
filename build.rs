fn main() {
    println!("cargo:rerun-if-changed=assets/app.manifest");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("assets/icon.ico");
    // Rust's GNU linker already supplies a default manifest; embedding a second
    // one only produces an ld merge warning. The MSVC release build (the one we
    // ship) gets the real manifest here. The GNU build still sets per-monitor
    // DPI awareness at runtime.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        resource.set_manifest_file("assets/app.manifest");
    }
    resource.set("ProductName", "D2R Load Remover");
    resource.set("FileDescription", "D2R Load Remover for LiveSplit");
    resource.set("OriginalFilename", "d2r-load-remover.exe");

    if let Err(error) = resource.compile() {
        // Resource embedding is a packaging concern; never fail a code build
        // because a resource compiler is unavailable in a minimal environment.
        println!("cargo:warning=could not embed Windows resources: {error}");
    }
}
