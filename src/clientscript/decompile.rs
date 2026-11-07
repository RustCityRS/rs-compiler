//! Stack reconstruction, type inference and structured source generation.
//! Cache traversal and name discovery belong to the embedding application.
pub(crate) mod inference;
mod source;
mod structure;
pub use source::{
    Signature, component_hook_text, decompile, infer_signatures, script_key, script_name,
    signatures,
};

pub trait Names {
    fn name(&self, kind: &str, id: u32) -> String;
    fn component_name(&self, interface: u32, component: u32) -> String {
        let _ = interface;
        format!("com{component}")
    }
}
