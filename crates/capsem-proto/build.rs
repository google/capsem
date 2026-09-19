//! Compile-time hash of normalized protocol syntax.
//!
//! Rust parsing removes comments, token rendering normalizes formatting, and
//! only type/constant declarations enter the digest. Documentation, rustfmt,
//! and implementation-only edits therefore do not make compatible binaries
//! refuse each other. Serde attributes, field names, variants and framing
//! constants remain in the token stream and change the hash.

use quote::ToTokens;
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
    let files = ["lib.rs", "ipc.rs", "handshake.rs", "router.rs"];
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
    std::fs::write(format!("{out_dir}/schema_hash.txt"), format!("{hash}u64"))
        .expect("schema hash: write generated constant");
}
