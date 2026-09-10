# OpenAPI for Zed

A [Zed extension](https://zed.dev/docs/extensions/developing-extensions) for OpenAPI specs.
`extension.toml` lives at the repo root, so Zed compiles the root crate to WASM and uses it to
start `openapi-lsp` over LSP.

None of the language logic lives in the WASM module (heavy parsing has no business running under
WASI). It is split into two ordinary crates instead:

| Path | Role |
| --- | --- |
| `extension.toml` + `src/lib.rs` | Zed extension: locate and launch `openapi-lsp` |
| `openapi-core/` | Document model: OAS detection, JSON Pointer, `$ref` resolution, offset/position math, JSON Schema lookup (no LSP) |
| `openapi-core/schemas/` | Bundled OpenAPI 2.0 / 3.0 / 3.1 JSON Schemas (provenance in `SOURCES.md`) |
| `openapi-lsp/` | Language server: `server.rs` implements `LanguageServer`, and `completion`/`hover`/`definition`/`diagnostics` each own one capability |

```
openapi-lsp/src
├── main.rs          tokio + LspService/Server, served over stdio
├── server.rs        capabilities, document sync, request dispatch (impl LanguageServer)
├── completion.rs    $ref target completion + schema-driven key/enum completion
├── hover.rs         $ref hover preview, documentHighlight
├── definition.rs    $ref go-to-definition, documentLink
└── diagnostics.rs   parse errors, unresolvable $ref/Pointer, JSON Schema validation
```

## Features

- **Go to Definition** and **document links** on `$ref`, including cross-file refs
- **Hover** preview of the node a `$ref` points at
- **Completion** for `$ref` targets, plus schema-driven keys and enum values
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
git tag lsp-v0.1.0 && git push origin lsp-v0.1.0
```

On an `lsp-v*` tag (or a manual `workflow_dispatch` against an existing tag) CI first runs clippy
(`-D warnings`) and the tests, then builds five targets and packages each one as
`openapi-lsp-<tag>-<target>.tar.gz` (`.zip` on Windows) with a sibling `.sha256`, and finally
creates or updates the GitHub release for that tag:

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

- **`$ref` values** come from the document itself (which components exist). Completion only fires
  when the container under the cursor matches the table in `openapi-core/src/keys.rs` — no match
  means no suggestions, rather than guessing `/components/schemas`. Accepting a completion replaces
  the whole `$ref: "..."` entry and keeps whichever quote style you already typed.
- **Keys and enum values** come from the bundled JSON Schemas. That is why schema keywords no
  longer pop up under user-named maps such as `properties`, `paths`, or `components/schemas`, while
  positions like `type:` and `in:` do offer enum values.

Diagnostics have the same source: unknown keys, missing required keys, enum/type mismatches. Nodes
whose value is `null` (i.e. you are still typing) are skipped so the file does not light up mid-keystroke.

All three schemas (2.0 / 3.0 / 3.1) are bundled under `openapi-core/schemas/`. The 3.1 Schema
Object delegates to the JSON Schema 2020-12 dialect via `$dynamicRef: "#meta"`, so that dialect and
its vocabularies are bundled too. `unevaluatedProperties` and `if`/`then` are not evaluated, which
makes the "unknown key" check slightly more permissive on 3.1 than on 3.0.

To debug, check `zed: open log`, or run `zed --foreground` to watch the server's
`window/logMessage` output.
