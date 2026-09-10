use zed_extension_api::{self as zed, Result};

const SERVER_BINARY: &str = "openapi-lsp";

struct OpenApiExtension;

impl zed::Extension for OpenApiExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        let command = worktree.which(SERVER_BINARY).ok_or_else(|| {
            format!(
                "{SERVER_BINARY} not found on PATH. From this repo run: \
                 cargo install --path openapi-lsp"
            )
        })?;

        Ok(zed::Command {
            command,
            args: Vec::new(),
            env: Default::default(),
        })
    }
}

zed::register_extension!(OpenApiExtension);
