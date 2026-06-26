//! Canonical Rust types for the `sqlite:extension/policy` WIT
//! contract.
//!
//! The types themselves — `Capability`, `HttpPolicy`, `DnsPolicy`,
//! `Policy`, `PolicyError` — are db-agnostic and now live in the
//! shared `datalink-policy` crate, which the sqlink and ducklink
//! wasm-component hosts both consume so a `Policy` value constructed
//! once is portable across deployment modes AND across hosts.
//!
//! This crate keeps the `sqlite-extension-policy` name and re-exports
//! the canonical types unchanged, so every existing
//! `sqlite_extension_policy::{...}` call site (loader-side hosts in
//! `sqlink-loader/runtimes/{wasmtime,wamr}` and the in-WASM
//! `sqlink/host`) compiles without modification. Each consumer's
//! crate-local `from_wit` conversion from its bindgen-generated
//! `LoadOptions` continues to live at the consumer; only the shared
//! types moved.

#![forbid(unsafe_code)]

pub use datalink_policy::{Capability, DnsPolicy, HttpPolicy, Policy, PolicyError};
