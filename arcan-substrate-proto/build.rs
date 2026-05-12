//! Build script for `arcan-substrate-proto`.
//!
//! Generates Rust types from `core/life/proto/arcan/v1/substrate.proto`
//! using `tonic-prost-build`. The proto root is three levels up from
//! the crate manifest dir (`crates/arcan/arcan-substrate-proto/` →
//! `core/life/`).
//!
//! Uses `extern_path` to reuse the canonical `aios.v1.*` types from
//! the `aios-proto` crate instead of regenerating them inside this
//! crate (Spec C₂ §10.3 — single Rust representation per wire type).

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let proto_root = manifest_dir
        .parent() // crates/arcan/
        .and_then(|p| p.parent()) // crates/
        .and_then(|p| p.parent()) // core/life/
        .ok_or("walking up to core/life/")?
        .join("proto");

    let proto_file = proto_root.join("arcan").join("v1").join("substrate.proto");
    println!("cargo:rerun-if-changed={}", proto_file.display());
    // Track the imported aios.v1.* sources too so a vocabulary edit
    // forces a regen.
    let aios_dir = proto_root.join("aios").join("v1");
    if let Ok(entries) = std::fs::read_dir(&aios_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("proto") {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        // Reuse the canonical aios.v1 types from aios-proto.
        .extern_path(".aios.v1", "::aios_proto::aios::v1")
        .compile_protos(&[proto_file], &[proto_root])?;

    Ok(())
}
