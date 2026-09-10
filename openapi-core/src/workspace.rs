//! In-memory store of the documents the editor has open.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::config::Settings;
use crate::document::Document;
use crate::resolve::Docs;

#[derive(Default)]
pub struct Workspace {
    docs: HashMap<String, Document>,
    settings: Settings,
    roots: Vec<PathBuf>,
}

impl Workspace {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the settings the editor sent; unset values return to default.
    pub fn set_settings(&mut self, settings: Settings) {
        self.settings = settings;
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Directories the editor has open. `$ref` file scanning stays inside
    /// them; empty means we fall back to sniffing out the checkout root.
    pub fn set_roots(&mut self, roots: Vec<PathBuf>) {
        self.roots = roots;
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub fn open(&mut self, uri: String, text: String, version: i32) {
        let previous = self.docs.get(&uri);
        let doc = Document::parse(uri.clone(), text, version, previous);
        self.docs.insert(uri, doc);
    }

    pub fn change(&mut self, uri: &str, text: String, version: i32) {
        self.open(uri.to_string(), text, version);
    }

    pub fn close(&mut self, uri: &str) {
        self.docs.remove(uri);
    }

    pub fn document(&self, uri: &str) -> Option<&Document> {
        self.docs.get(uri)
    }

    /// Read-only view of every open document, used by `$ref` resolution.
    pub fn docs(&self) -> Docs<'_> {
        Docs::new(&self.docs)
    }
}
