fn main() {
    println!("cargo:rerun-if-changed=native/ma_shim.c");
    println!("cargo:rerun-if-changed=native/miniaudio.h");

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
}
