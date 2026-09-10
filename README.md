# OpenAPI for Zed

A [Zed extension](https://zed.dev/docs/extensions/developing-extensions) for OpenAPI specs.
`extension.toml` lives at the repo root, so Zed compiles the root crate to WASM and uses it to
start `openapi-lsp` over LSP.

None of the language logic lives in the WASM module (heavy parsing has no business running under
WASI). It is split into two ordinary crates instead:

| Path | Role |
| --- | --- |
| `extension.toml` + `src/lib.rs` | Zed extension: locate and launch `openapi-lsp` |
| `openapi-core/` | Document model: OAS detection, JSON Pointer, `$ref` resolution, neighbouring-file discovery, settings, offset/position math, JSON Schema lookup (no LSP) |
| `openapi-core/schemas/` | Bundled OpenAPI 2.0 / 3.0 / 3.1 JSON Schemas (provenance in `SOURCES.md`) |
| `openapi-lsp/` | Language server: `server.rs` implements `LanguageServer`, and `completion`/`hover`/`definition`/`diagnostics` each own one capability |

```
openapi-lsp/src
├── main.rs          tokio + LspService/Server, served over stdio
├── server.rs        capabilities, document sync, request dispatch (impl LanguageServer)
├── completion.rs    $ref target completion (local, cross-file, file paths) + schema-driven key/enum completion
├── hover.rs         $ref hover preview, documentHighlight
├── definition.rs    $ref go-to-definition, documentLink
└── diagnostics.rs   parse errors, unresolvable $ref/Pointer, JSON Schema validation
```

## Features

- **Go to Definition** and **document links** on `$ref`, including cross-file refs
- **Hover** preview of the node a `$ref` points at
- **Completion** for `$ref` targets — components in this file, files around it, and the pointers
  inside those files — plus schema-driven keys and enum values (`$ref` included)
- **Diagnostics** for parse errors, unresolvable `$ref`/JSON Pointer, and JSON Schema violations
- YAML and JSON specs, OpenAPI 2.0 / 3.0 / 3.1

## Install

1. Put the language server on your `PATH`. Either download an archive from
   [Releases](https://github.com/Dyqer/openapi/releases) and unpack the binary into a directory on
   your `PATH`, or build it yourself:

   ```bash
   cargo install --path openapi-lsp
   # verify
   which openapi-lsp
   ```

2. Install the extension in Zed:

   - Command Palette → `zed: install dev extension`
   - Select the **repo root** (the directory containing `extension.toml`)

3. Open an OpenAPI YAML/JSON file and try Go to Definition on a `$ref`.

Zed builds the extension with `cargo build --target wasm32-wasip2`, so the workspace's
`default-members` contains only the root crate — `openapi-core` and `openapi-lsp` are never pulled
into the WASM build. If your Rust toolchain does not come from rustup, install that target
yourself.

## Development

```bash
# Only the two ordinary crates are testable (the root crate is the wasm extension)
cargo test -p openapi-core -p openapi-lsp

# The extension itself holds almost no logic; Zed compiles this WASM on dev-extension install
cargo build --target wasm32-wasip2
```

## Releasing the language server

`.github/workflows/lsp-release.yml` builds the `openapi-lsp` release artifacts:

```bash
git tag lsp-v0.2.0 && git push origin lsp-v0.2.0
```

On an `lsp-v*` tag (or a manual `workflow_dispatch` against an existing tag) CI first runs clippy
(`-D warnings`) and the tests, then builds five targets and packages each one as
`openapi-lsp-<version>-<target>.tar.gz` (`.zip` on Windows) with a sibling `.sha256`, and finally
creates or updates the GitHub release for that tag. `<version>` is the tag without its `lsp-`
prefix, so `lsp-v0.2.0` produces `openapi-lsp-v0.2.0-aarch64-apple-darwin.tar.gz`:

| Target | Runner |
| --- | --- |
| `x86_64-unknown-linux-gnu` | `ubuntu-latest` |
| `aarch64-unknown-linux-gnu` | `ubuntu-24.04-arm` |
| `x86_64-apple-darwin` | `macos-latest` |
| `aarch64-apple-darwin` | `macos-latest` |
| `x86_64-pc-windows-msvc` | `windows-latest` |

The extension only ever looks for `openapi-lsp` on `PATH` (`src/lib.rs`), so unpacking a release
archive somewhere on `PATH` is enough — no `cargo install` required.

## Where completions come from

Completions come from two independent sources:

- **`$ref` values** come from the document itself (which components exist) and from the files
  around it. Completion only fires when the container under the cursor matches the table in
  `openapi-core/src/keys.rs` — no match means no suggestions, rather than guessing
  `/components/schemas`. Accepting a completion replaces the whole `$ref: "..."` entry and keeps
  whichever quote style you already typed. What you get depends on what you have typed:

  | Typed | Offered |
  | --- | --- |
  | nothing yet | both lists below, this file's own components first |
  | `#…` | components in this file: `#/components/schemas/Pet` |
  | `./pets.yaml#…` | components in that file — its component map, or its top level when it is a standalone schema file with no `/components/schemas` |
  | anything else | components in the files around this one, as the whole reference: `./schemas/pets.yaml#/components/schemas/Pet`. Bare file names are never offered, and a neighbour that has no component map contributes nothing — naming it explicitly is what falls back to its top level |

  The list reads name-first — `Pet`, then a dim `pet.yaml`, with the whole
  `../../shared/schemas/pet.yaml#/components/schemas/Pet` as the item's description. Leading with
  the reference would bury the name behind a row of `../..` at the popup's width. Filtering still
  runs against the full text, so either the name or the path narrows the list.

  Keys are filtered on the server when the word under the cursor starts with `$`, so `$r` offers
  `$ref` alone. Clients fuzzy-match on word characters and would otherwise still rank `readOnly`
  and `required` into that list.

- **Keys and enum values** come from the bundled JSON Schemas. That is why schema keywords no
  longer pop up under user-named maps such as `properties`, `paths`, or `components/schemas`, while
  positions like `type:` and `in:` do offer enum values. `$ref` itself is offered wherever a
  Reference Object fits, which takes two workarounds: the 3.0 schema declares it through
  `patternProperties: {"^\\$ref$": …}`, and the 3.1 schema hides it behind `if`/`then`
  (`openapi-core/src/jsonschema.rs` handles both).

## Settings

Which files `$ref` completion offers is configurable. In Zed, put them under
`lsp.openapi-lsp.initialization_options` in `settings.json` (a `workspace/didChangeConfiguration`
payload with the same shape works too):

```json
{
  "lsp": {
    "openapi-lsp": {
      "initialization_options": {
        "openapi": {
          "refFiles": {
            "scanUp": 6,
            "scanDown": 6,
            "maxFiles": 500,
            "extensions": [],
            "skipDirs": ["node_modules", "target", "dist", "build", "vendor"]
          }
        }
      }
    }
  }
}
```

| Key | Default | Meaning |
| --- | --- | --- |
| `scanUp` | `6` | Directory levels above the document to include |
| `scanDown` | `6` | Directory levels below the document's own directory to descend into |
| `maxFiles` | `500` | Upper bound on offered files |
| `extensions` | `[]` | Extensions to scan; empty means "the same suffix as the file being edited", so a YAML spec never suggests its own JSON build output |
| `skipDirs` | see above | Directory names never scanned, whatever their depth |

The directories Zed has open are a hard ceiling: whatever `scanUp` says, the scan never leaves the
worktree. With no workspace folder to go on (a single file opened on its own) it stops at the
enclosing `.git`/`.hg` checkout instead. Dotted directories and symlinks are never followed, and a
scan is reused for two seconds so a burst of keystrokes costs one walk. Each neighbour is parsed
once and remembered until its size or modification time changes; open files are read from the
editor's buffer instead, so unsaved components show up.

Diagnostics have the same source: unknown keys, missing required keys, enum/type mismatches. Nodes
whose value is `null` (i.e. you are still typing) are skipped so the file does not light up mid-keystroke.

All three schemas (2.0 / 3.0 / 3.1) are bundled under `openapi-core/schemas/`. The 3.1 Schema
Object delegates to the JSON Schema 2020-12 dialect via `$dynamicRef: "#meta"`, so that dialect and
its vocabularies are bundled too. `if`/`then`/`else` steers completion but is not enforced by
validation, and `unevaluatedProperties` is ignored entirely, which makes the "unknown key" check
slightly more permissive on 3.1 than on 3.0.

To debug, check `zed: open log`, or run `zed --foreground` to watch the server's
`window/logMessage` output.
