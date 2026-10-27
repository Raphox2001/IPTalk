fn main() {
    println!("cargo:rerun-if-changed=native/ma_shim.c");
    println!("cargo:rerun-if-changed=native/miniaudio.h");
    println!("cargo:rerun-if-changed=assets/iptalk.ico");

    let mut build = cc::Build::new();
    build
        .file("native/ma_shim.c")
        .include("native")
        .opt_level(2)
        .warnings(false)
        // Nur die Backends, die IPTalk wirklich nutzt.
        .define("MA_NO_DECODING", None)
        .define("MA_NO_ENCODING", None)
        .define("MA_NO_GENERATION", None)
        .define("MA_NO_RESOURCE_MANAGER", None)
        .define("MA_NO_NODE_GRAPH", None)
        .define("MA_NO_ENGINE", None);

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "linux" {
        // PulseAudio/PipeWire, ALSA und JACK werden zur Laufzeit per dlopen geladen,
        // es werden also keine -dev-Pakete zum Bauen gebraucht.
        println!("cargo:rustc-link-lib=dl");
        println!("cargo:rustc-link-lib=pthread");
        println!("cargo:rustc-link-lib=m");
    }
    build.compile("ma_shim");

    // Windows: Icon und Versionsinformationen (Explorer, Task-Manager, Eigenschaften).
    #[cfg(windows)]
    if target_os == "windows" {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/iptalk.ico")
            .set("ProductName", "IPTalk")
            .set("FileDescription", "IPTalk – Sprachverbindung mit minimaler Latenz")
            .set("CompanyName", "Raphox2001")
            .set("LegalCopyright", "© 2026 Raphox2001, MIT-Lizenz")
            .set("OriginalFilename", "IPTalk.exe")
            .set("InternalName", "IPTalk");
        if let Err(e) = res.compile() {
            println!("cargo:warning=Windows-Ressourcen konnten nicht eingebunden werden: {e}");
        }
    }
}
