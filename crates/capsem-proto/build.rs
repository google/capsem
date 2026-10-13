//! Compile-time hash of normalized protocol syntax.
//!
//! Rust parsing removes comments, token rendering normalizes formatting, and
//! only type/constant declarations enter the digest. Documentation, rustfmt,
//! and implementation-only edits therefore do not make compatible binaries
//! refuse each other. Serde attributes, field names, variants and framing
//! constants remain in the token stream and change the hash.

use quote::ToTokens;
use std::path::PathBuf;
use syn::visit_mut::VisitMut;

struct StripDocs;

impl VisitMut for StripDocs {
    fn visit_attributes_mut(&mut self, attributes: &mut Vec<syn::Attribute>) {
        attributes.retain(|attribute| !attribute.path().is_ident("doc"));
        for attribute in attributes {
            syn::visit_mut::visit_attribute_mut(self, attribute);
        }
    }
}

fn main() {
    let files = ["lib.rs", "ipc.rs", "handshake.rs", "router.rs", "exec_stream.rs"];
    write_hash("schema_hash.txt", &files);
    write_hash(
        "ledger_schema_hash.txt",
        &["ledger.rs", "ledger_commitment.rs", "ledger_control.rs"],
    );
    write_hash("proxy_schema_hash.txt", &["proxy_control.rs", "proxy_policy.rs"]);
    write_runtime_limits();
}

fn write_runtime_limits() {
    let config_path = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"))
        .join("../../config/data/runtime-limits.toml");
    let source = std::fs::read_to_string(&config_path)
        .unwrap_or_else(|error| panic!("runtime limits: read {}: {error}", config_path.display()));
    let config: toml::Value = toml::from_str(&source)
        .unwrap_or_else(|error| panic!("runtime limits: parse {}: {error}", config_path.display()));
    let capacity = config
        .get("network")
        .and_then(|network| network.get("max_concurrent_clients"))
        .and_then(toml::Value::as_integer)
        .filter(|value| (1..=4096).contains(value))
        .unwrap_or_else(|| {
            panic!("runtime limits: network.max_concurrent_clients must be an integer between 1 and 4096")
        });
    let descriptors_per_client = config
        .get("network")
        .and_then(|network| network.get("file_descriptors_per_client"))
        .and_then(toml::Value::as_integer)
        .filter(|value| (3..=64).contains(value))
        .unwrap_or_else(|| {
            panic!("runtime limits: network.file_descriptors_per_client must be an integer between 3 and 64")
        });
    let open_file_limit = capacity
        .checked_mul(descriptors_per_client)
        .filter(|value| *value <= 1_048_576)
        .expect("runtime limits: derived network descriptor budget exceeds 1048576");
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    std::fs::write(
        format!("{out_dir}/runtime_limits.rs"),
        format!(
            "/// Supported concurrent network-client ceiling for one VM.\n\
             ///\n\
             /// TCP descriptor grants and each of the two DNS sessions derive\n\
             /// their bounds from this one config-owned value.\n\
             pub const NETWORK_CLIENT_CAPACITY: usize = {capacity};\n\
             /// Minimum descriptor budget for a network-facing process.\n\
             pub const NETWORK_PROCESS_OPEN_FILE_LIMIT: u64 = {open_file_limit};\n"
        ),
    )
    .expect("runtime limits: write generated constants");
    println!("cargo:rerun-if-changed={}", config_path.display());
}

fn write_hash(output: &str, files: &[&str]) {
    let mut hash = 0xcbf29ce484222325_u64;

    for f in files {
        let path = format!("src/{f}");
        let source = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("schema hash: read {path}: {error}"));
        let mut syntax = syn::parse_file(&source).unwrap_or_else(|error| panic!("schema hash: parse {path}: {error}"));
        StripDocs.visit_file_mut(&mut syntax);
        for item in syntax.items {
            if matches!(
                &item,
                syn::Item::Const(_)
                    | syn::Item::Enum(_)
                    | syn::Item::Static(_)
                    | syn::Item::Struct(_)
                    | syn::Item::Type(_)
                    | syn::Item::Union(_)
            ) {
                for byte in item.into_token_stream().to_string().bytes() {
                    hash ^= u64::from(byte);
                    hash = hash.wrapping_mul(0x100000001b3);
                }
            }
        }
        println!("cargo:rerun-if-changed={path}");
    }

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    std::fs::write(format!("{out_dir}/{output}"), format!("{hash}u64")).expect("schema hash: write generated constant");
}
