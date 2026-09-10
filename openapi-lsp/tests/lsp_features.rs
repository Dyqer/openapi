use openapi_core::{Position, Workspace};
use openapi_lsp::{completion, definition, diagnostics, hover};
use tower_lsp::lsp_types::{GotoDefinitionResponse, LocationLink, Range};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(fixture_path(name)).unwrap()
}

fn fixture_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn file_uri(name: &str) -> String {
    format!("file://{}", fixture_path(name).display())
}

fn cursor_in(text: &str, needle: &str) -> Position {
    let byte = text.find(needle).expect(needle);
    openapi_core::pos::offset_to_position(text, byte + needle.len() / 2)
}

fn open(name: &str) -> (Workspace, String, String) {
    let mut ws = Workspace::new();
    let uri = file_uri(name);
    let text = fixture(name);
    ws.open(uri.clone(), text.clone(), 1);
    (ws, uri, text)
}

fn link(ws: &Workspace, uri: &str, pos: Position) -> LocationLink {
    match definition::goto_definition(ws, uri, pos).expect("definition") {
        GotoDefinitionResponse::Link(mut links) => links.remove(0),
        other => panic!("expected a link response, got {other:?}"),
    }
}

fn span(text: &str, range: Range) -> String {
    let line = text.lines().nth(range.start.line as usize).unwrap();
    line[range.start.character as usize..range.end.character as usize].to_string()
}

#[test]
fn detects_openapi_and_ignores_plain_yaml() {
    let (mut ws, uri, _) = open("petstore.yaml");
    assert!(ws.document(&uri).unwrap().version_kind.is_openapi());

    let plain = "file:///tmp/compose.yaml".to_string();
    ws.open(plain.clone(), "services:\n  web:\n    image: nginx\n".into(), 1);
    assert!(!ws.document(&plain).unwrap().version_kind.is_openapi());
    assert!(completion::completion(&ws, &plain, Position::new(0, 0)).is_empty());
    assert!(diagnostics::diagnostics(&ws, &plain).is_empty());
}

#[test]
fn jumps_to_local_schema() {
    let (ws, uri, text) = open("petstore.yaml");
    let pos = cursor_in(&text, "#/components/schemas/Pet");
    let link = link(&ws, &uri, pos);
    let line = text
        .lines()
        .nth(link.target_range.start.line as usize)
        .unwrap();
    assert!(line.contains("Pet:"), "jumped to `{line}`");
}

#[test]
fn cmd_click_highlights_entire_ref_value() {
    let (ws, uri, text) = open("petstore.yaml");
    let pos = cursor_in(&text, "#/components/schemas/Pet");

    let link = link(&ws, &uri, pos);
    let origin = link.origin_selection_range.expect("origin range");
    assert_eq!(span(&text, origin), "\"#/components/schemas/Pet\"");

    let highlights = hover::document_highlights(&ws, &uri, pos);
    assert_eq!(span(&text, highlights[0].range), "\"#/components/schemas/Pet\"");

    let links = definition::document_links(&ws, &uri);
    assert!(
        links.iter().any(|l| {
            let span = span(&text, l.range);
            span.contains("#/components/schemas/Pet") && span.starts_with('"')
        }),
        "document links should cover the full quoted $ref"
    );
}

#[test]
fn jumps_to_external_file() {
    let (ws, uri, text) = open("petstore.yaml");
    let pos = cursor_in(&text, "./pet.yaml#/PetName");
    let link = link(&ws, &uri, pos);
    assert!(
        link.target_uri.as_str().ends_with("pet.yaml"),
        "{}",
        link.target_uri
    );
    let target = fixture("pet.yaml");
    let line = target
        .lines()
        .nth(link.target_range.start.line as usize)
        .unwrap();
    assert!(line.contains("PetName"));
}

#[test]
fn hover_shows_the_resolved_node() {
    let (ws, uri, text) = open("petstore.yaml");
    let pos = cursor_in(&text, "#/components/schemas/Pet");
    let hover = hover::hover(&ws, &uri, pos).expect("hover");
    let rendered = format!("{:?}", hover.contents);
    assert!(rendered.contains("$ref"), "{rendered}");
    assert!(rendered.contains("```yaml"), "{rendered}");
}

#[test]
fn completes_ref_schema_names() {
    let (ws, uri, text) = open("petstore.yaml");
    let pos = cursor_in(&text, "#/components/schemas/Pet");
    let labels: Vec<_> = completion::completion(&ws, &uri, pos)
        .into_iter()
        .map(|c| c.label)
        .collect();
    assert!(labels.iter().any(|l| l.contains("Pet")), "{labels:?}");
    assert!(labels.iter().any(|l| l.contains("Error")), "{labels:?}");
}

#[test]
fn completes_root_keys() {
    let mut ws = Workspace::new();
    let uri = "file:///tmp/new.yaml".to_string();
    ws.open(
        uri.clone(),
        "openapi: \"3.0.3\"\ninfo:\n  title: t\n  version: \"1\"\n\n".into(),
        1,
    );
    let labels: Vec<_> = completion::completion(&ws, &uri, Position::new(4, 0))
        .into_iter()
        .map(|c| c.label)
        .collect();
    assert!(
        labels.contains(&"paths".into()) || labels.contains(&"components".into()),
        "{labels:?}"
    );
}

#[test]
fn flags_missing_pointer() {
    let mut ws = Workspace::new();
    let uri = "file:///tmp/bad.yaml".to_string();
    ws.open(
        uri.clone(),
        "openapi: 3.0.3\ninfo:\n  title: t\n  version: '1'\npaths: {}\ncomponents:\n  schemas:\n    A:\n      $ref: '#/components/schemas/Missing'\n".into(),
        1,
    );
    let diags = diagnostics::diagnostics(&ws, &uri);
    assert!(
        diags.iter().any(|d| d.message.contains("Missing")),
        "{diags:?}"
    );
}

#[test]
fn json_local_ref_jump() {
    let (ws, uri, text) = open("petstore.json");
    let pos = cursor_in(&text, "#/components/schemas/Pet");
    let link = link(&ws, &uri, pos);
    let line = text
        .lines()
        .nth(link.target_range.start.line as usize)
        .unwrap();
    assert!(line.contains("Pet"), "{line}");
}

// --- schema-driven key/enum completion -------------------------------------

const HEAD: &str = "openapi: 3.0.3\ninfo:\n  title: t\n  version: '1'\n";

/// Open a document whose cursor position is marked with `▮`.
fn at_cursor(text: &str) -> (Workspace, String, Position) {
    let offset = text.find('▮').expect("cursor marker");
    let text = text.replace('▮', "");
    let pos = openapi_core::pos::offset_to_position(&text, offset);
    let mut ws = Workspace::new();
    let uri = "file:///tmp/cursor.yaml".to_string();
    ws.open(uri.clone(), text, 1);
    (ws, uri, pos)
}

fn labels_at(text: &str) -> Vec<String> {
    let (ws, uri, pos) = at_cursor(text);
    completion::completion(&ws, &uri, pos)
        .into_iter()
        .map(|c| c.label)
        .collect()
}

#[test]
fn user_named_maps_offer_no_keys() {
    // Property names are the user's own, so there is nothing to suggest.
    let inside_properties = labels_at(&format!(
        "{HEAD}paths: {{}}\ncomponents:\n  schemas:\n    Pet:\n      properties:\n        ▮\n"
    ));
    assert!(inside_properties.is_empty(), "{inside_properties:?}");

    // Same for path names and component names.
    let inside_paths = labels_at(&format!("{HEAD}paths:\n  ▮\n"));
    assert!(inside_paths.is_empty(), "{inside_paths:?}");

    let inside_schemas =
        labels_at(&format!("{HEAD}paths: {{}}\ncomponents:\n  schemas:\n    ▮\n"));
    assert!(inside_schemas.is_empty(), "{inside_schemas:?}");
}

#[test]
fn schema_drives_key_completion() {
    let root = labels_at(&format!("{HEAD}▮\n"));
    assert!(root.contains(&"paths".into()), "{root:?}");
    assert!(root.contains(&"components".into()), "{root:?}");
    // Keys already written are not offered again.
    assert!(!root.contains(&"info".into()), "{root:?}");

    let operation = labels_at(&format!(
        "{HEAD}paths:\n  /pets:\n    get:\n      responses: {{}}\n      ▮\n"
    ));
    assert!(operation.contains(&"operationId".into()), "{operation:?}");
    assert!(operation.contains(&"parameters".into()), "{operation:?}");
    assert!(!operation.contains(&"type".into()), "{operation:?}");

    let schema = labels_at(&format!(
        "{HEAD}paths: {{}}\ncomponents:\n  schemas:\n    Pet:\n      type: object\n      ▮\n"
    ));
    assert!(schema.contains(&"format".into()), "{schema:?}");
    assert!(schema.contains(&"required".into()), "{schema:?}");
    assert!(!schema.contains(&"operationId".into()), "{schema:?}");
    assert!(!schema.contains(&"type".into()), "{schema:?}");
}

#[test]
fn completes_enum_values() {
    let types = labels_at(&format!(
        "{HEAD}paths: {{}}\ncomponents:\n  schemas:\n    Pet:\n      type: ▮\n"
    ));
    assert!(types.contains(&"string".into()), "{types:?}");
    assert!(types.contains(&"integer".into()), "{types:?}");

    let parameter_in = labels_at(&format!(
        "{HEAD}paths:\n  /pets:\n    get:\n      parameters:\n        - name: id\n          in: ▮\n"
    ));
    assert!(parameter_in.contains(&"query".into()), "{parameter_in:?}");
}

#[test]
fn ref_completion_keeps_quote_style_and_replaces_the_whole_pair() {
    let text = format!(
        "{HEAD}paths: {{}}\ncomponents:\n  schemas:\n    Pet:\n      type: object\n    Wrap:\n      properties:\n        pet:\n          $ref: '#/comp▮'\n"
    );
    let (ws, uri, pos) = at_cursor(&text);
    let items = completion::completion(&ws, &uri, pos);
    let item = items
        .iter()
        .find(|i| i.label == "#/components/schemas/Pet")
        .unwrap_or_else(|| panic!("{:?}", items.iter().map(|i| &i.label).collect::<Vec<_>>()));

    // Single quotes were typed, so single quotes come back.
    assert_eq!(
        item.insert_text.as_deref(),
        Some("$ref: '#/components/schemas/Pet'")
    );
    // The edit covers `$ref: '#/comp'` — key included, so both stay consistent.
    let edit = match item.text_edit.as_ref().expect("text edit") {
        tower_lsp::lsp_types::CompletionTextEdit::Edit(edit) => edit,
        other => panic!("{other:?}"),
    };
    let stripped = text.replace('▮', "");
    let line = stripped.lines().nth(edit.range.start.line as usize).unwrap();
    assert_eq!(
        &line[edit.range.start.character as usize..edit.range.end.character as usize],
        "$ref: '#/comp'"
    );
}

#[test]
fn no_ref_completion_in_unknown_containers() {
    // `info.contact` holds no components, so we do not guess a container.
    let items = labels_at(&format!("{HEAD}  contact:\n    $ref: '#/▮'\npaths: {{}}\n"));
    assert!(items.is_empty(), "{items:?}");
}

#[test]
fn schema_violations_are_reported() {
    let mut ws = Workspace::new();
    let uri = "file:///tmp/bad-schema.yaml".to_string();
    ws.open(
        uri.clone(),
        format!("{HEAD}  licence: MIT\npaths:\n  /pets:\n    get:\n      responses:\n        '200':\n          description: ok\n      deprecated: yes-please\n"),
        1,
    );
    let diags = diagnostics::diagnostics(&ws, &uri);
    let messages: Vec<&str> = diags.iter().map(|d| d.message.as_str()).collect();
    assert!(
        messages.iter().any(|m| m.contains("`licence` is not allowed")),
        "{messages:?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("expected boolean")),
        "{messages:?}"
    );
    // The unknown key is flagged where it is written, not at the file start.
    let licence = diags
        .iter()
        .find(|d| d.message.contains("licence"))
        .expect("licence diagnostic");
    assert_eq!(licence.range.start.line, 4);
}

#[test]
fn valid_spec_has_no_schema_noise() {
    let mut ws = Workspace::new();
    let uri = "file:///tmp/ok.yaml".to_string();
    ws.open(
        uri.clone(),
        format!("{HEAD}paths:\n  /pets:\n    get:\n      responses:\n        '200':\n          description: ok\ncomponents:\n  schemas:\n    Pet:\n      type: object\n      properties:\n        id:\n          type: integer\n"),
        1,
    );
    let diags = diagnostics::diagnostics(&ws, &uri);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn openapi_3_1_is_schema_driven_too() {
    // 3.1's root schema carries a `$ref` next to its `properties`, and its
    // Schema Objects delegate to the JSON Schema 2020-12 dialect.
    let head = "openapi: 3.1.0\ninfo:\n  title: t\n  version: '1'\n";

    let root = labels_at(&format!("{head}▮\n"));
    assert!(root.contains(&"paths".into()), "{root:?}");
    assert!(root.contains(&"webhooks".into()), "{root:?}");

    let schema = labels_at(&format!(
        "{head}paths: {{}}\ncomponents:\n  schemas:\n    Pet:\n      type: object\n      ▮\n"
    ));
    assert!(schema.contains(&"properties".into()), "{schema:?}");
    assert!(schema.contains(&"required".into()), "{schema:?}");
    assert!(schema.contains(&"prefixItems".into()), "{schema:?}");

    // Property names stay free-form here as well.
    let names = labels_at(&format!(
        "{head}paths: {{}}\ncomponents:\n  schemas:\n    Pet:\n      properties:\n        ▮\n"
    ));
    assert!(names.is_empty(), "{names:?}");
}

#[test]
fn openapi_3_1_spec_has_no_schema_noise() {
    let mut ws = Workspace::new();
    let uri = "file:///tmp/ok31.yaml".to_string();
    ws.open(
        uri.clone(),
        "openapi: 3.1.0\ninfo:\n  title: t\n  version: '1'\npaths:\n  /pets:\n    get:\n      responses:\n        '200':\n          description: ok\n          content:\n            application/json:\n              schema:\n                $ref: '#/components/schemas/Pet'\ncomponents:\n  schemas:\n    Pet:\n      type: object\n      properties:\n        id:\n          type: integer\n      required:\n        - id\n".into(),
        1,
    );
    let diags = diagnostics::diagnostics(&ws, &uri);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn completion_replaces_the_typed_word_and_keeps_indentation() {
    let text = "openapi: 3.0.3\ninfo:\n  title: t\n  version: '1'\npaths: {}\ncomponents:\n  schemas:\n    Pet:\n      typ▮\n";
    let (ws, uri, pos) = at_cursor(text);
    let item = completion::completion(&ws, &uri, pos)
        .into_iter()
        .find(|i| i.label == "type")
        .expect("type key");

    // No indentation baked into the text: the line already has it.
    assert_eq!(item.insert_text.as_deref(), Some("type: "));
    let edit = match item.text_edit.as_ref().expect("text edit") {
        tower_lsp::lsp_types::CompletionTextEdit::Edit(edit) => edit,
        other => panic!("{other:?}"),
    };
    // The edit covers exactly `typ`, so the six leading spaces survive.
    assert_eq!(edit.range.start.line, 8);
    assert_eq!(edit.range.start.character, 6);
    assert_eq!(edit.range.end.character, 9);
    assert_eq!(edit.new_text, "type: ");

    // Enum values replace the typed prefix too.
    let value = "openapi: 3.0.3\ninfo:\n  title: t\n  version: '1'\npaths: {}\ncomponents:\n  schemas:\n    Pet:\n      type: str▮\n";
    let (ws, uri, pos) = at_cursor(value);
    let item = completion::completion(&ws, &uri, pos)
        .into_iter()
        .find(|i| i.label == "string")
        .expect("string value");
    let edit = match item.text_edit.as_ref().expect("text edit") {
        tower_lsp::lsp_types::CompletionTextEdit::Edit(edit) => edit,
        other => panic!("{other:?}"),
    };
    assert_eq!(edit.range.start.character, 12);
    assert_eq!(edit.range.end.character, 15);
    assert_eq!(edit.new_text, "string");
}

#[test]
fn a_file_that_never_parsed_still_completes() {
    // Opened while broken: no parse, no `last_good` — the version is sniffed
    // from the text and the schema alone drives completion.
    let text = "openapi: 3.0.3\ninfo:\n  title: t\n  version: '1'\npaths:\n  /pets:\n    get:\n      oper▮\n      responses: {oops\n";
    let (ws, uri, pos) = at_cursor(text);
    let doc = ws.document(&uri).expect("document");
    assert!(doc.value.is_none(), "expected a parse failure");
    assert!(doc.last_good.is_none());
    assert!(doc.version_kind.is_openapi(), "{:?}", doc.version_kind);

    let labels: Vec<String> = completion::completion(&ws, &uri, pos)
        .into_iter()
        .map(|i| i.label)
        .collect();
    assert!(labels.contains(&"operationId".into()), "{labels:?}");
}
