//! In-memory store of the documents the editor has open.

use std::collections::HashMap;

use crate::document::Document;
use crate::resolve::Docs;

#[derive(Default)]
pub struct Workspace {
    docs: HashMap<String, Document>,
}

impl Workspace {
    pub fn new() -> Self {
        Self::default()
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
