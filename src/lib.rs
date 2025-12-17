//! `v8_hts` exposes HTSlib (VCF/BCF) records to V8.
//!
//! The primary entrypoint is [`runner::run_vcf_expr_to_stdout`], which iterates
//! records in a VCF/BCF and evaluates a JavaScript expression for each record.

pub mod header;
pub mod runtime;
pub mod runner;
pub mod variant;

pub use header::Header;
pub use variant::Variant;
