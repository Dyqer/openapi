//! OpenAPI document model shared by every editor front end.
//!
//! This crate knows about OpenAPI/Swagger specs, JSON Pointers and source
//! positions, but nothing about the Language Server Protocol. The protocol
//! layer lives in the `openapi-lsp` crate.

pub mod config;
pub mod detect;
pub mod document;
pub mod files;
pub mod jsonschema;
pub mod keys;
pub mod locate;
pub mod pointer;
pub mod pos;
pub mod refs;
pub mod resolve;
pub mod schemas;
pub mod workspace;

pub use config::{RefFileSettings, Settings};
pub use detect::OpenApiVersion;
pub use document::{Document, ParseFailure};
pub use jsonschema::{Issue, KeyInfo, Schema};
pub use pos::{Location, Position, Range};
pub use refs::{Ref, RefHit};
pub use resolve::{Docs, RefTarget};
pub use workspace::Workspace;
