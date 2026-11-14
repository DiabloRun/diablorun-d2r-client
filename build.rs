fn main() {
    println!("cargo:rerun-if-changed=assets/app.manifest");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("assets/icon.ico");
    // The release build is normally the GNU target, which otherwise ships
    // Rust's minimal default manifest. Embed the real manifest so the app gets
    // per-monitor DPI awareness and themed common controls. WinLibs/ld may warn
    // about multiple non-default manifests because it also links its own
    // default-manifest.o; the custom manifest still wins.
    resource.set_manifest_file("assets/app.manifest");
    resource.set("ProductName", "D2R LRT Tracker");
    resource.set("FileDescription", "D2R LRT Tracker for LiveSplit");
    resource.set("OriginalFilename", "d2r-lrt-tracker.exe");

    if let Err(error) = resource.compile() {
        // Resource embedding is a packaging concern; never fail a code build
        // because a resource compiler is unavailable in a minimal environment.
        println!("cargo:warning=could not embed Windows resources: {error}");
    }
}
