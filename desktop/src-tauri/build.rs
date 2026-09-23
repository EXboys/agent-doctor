use std::path::PathBuf;

fn main() {
    // Bake AGENT_DOCTOR_EDITION into agent-doctor-core via option_env! when set.
    println!("cargo:rerun-if-env-changed=AGENT_DOCTOR_EDITION");

    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let info_plist = manifest_dir.join("Info.plist");
    println!("cargo:rerun-if-changed={}", info_plist.display());

    // Stamp Info.plist into OUT_DIR so lib.rs can include_bytes! it. Changing the
    // plist then always recompiles the crate and re-runs tauri::generate_context!,
    // which embeds privacy usage strings for `tauri:dev` bare binaries.
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let stamp_path = out_dir.join("info_plist.stamp");
    let stamp = if info_plist.exists() {
        let bytes = std::fs::read(&info_plist).unwrap_or_default();
        format!("{}\n{:x}", bytes.len(), fnv1a64(&bytes))
    } else {
        String::from("missing")
    };
    std::fs::write(&stamp_path, stamp).expect("write info_plist.stamp");

    // `tauri:dev` (--no-default-features) lets generate_context embed Info.plist.
    // With `custom-protocol` (default / packaged bare runs), Tauri skips that embed.
    // Link the plist section ourselves so TCC still finds the usage descriptions.
    #[cfg(target_os = "macos")]
    {
        if std::env::var_os("CARGO_FEATURE_CUSTOM_PROTOCOL").is_some() && info_plist.exists() {
            println!("cargo:rustc-link-arg=-sectcreate");
            println!("cargo:rustc-link-arg=__TEXT");
            println!("cargo:rustc-link-arg=__info_plist");
            println!("cargo:rustc-link-arg={}", info_plist.display());
        }
    }

    tauri_build::build()
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
