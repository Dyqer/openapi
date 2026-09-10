//! LSP front end for [`openapi_core`].
//!
//! `server.rs` owns the protocol (`impl LanguageServer`); every feature module
//! next to it turns core results into `lsp_types` payloads and stays free of
//! transport concerns so it can be unit tested directly.

pub mod completion;
pub mod definition;
pub mod diagnostics;
pub mod hover;
pub mod server;

pub use server::Backend;
