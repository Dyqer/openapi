//! Zed extension entry point.
//!
//! The language server is a separate binary. A local build on `PATH` always
//! wins — that is what you want while developing — and otherwise the matching
//! release archive is downloaded from GitHub, so an ordinary user needs no
//! Rust toolchain.

use zed_extension_api::{
    self as zed, Architecture, DownloadedFileType, GithubReleaseOptions, LanguageServerId,
    LanguageServerInstallationStatus, Os, Result, Worktree, settings::LspSettings,
};

const SERVER_BINARY: &str = "openapi-lsp";
const REPO: &str = "Dyqer/openapi";
/// Every release asset is served from here; anything else means the API
/// response was not what we asked for, so we refuse to execute it.
const GITHUB_RELEASE_PREFIX: &str = "https://github.com/Dyqer/openapi/releases/download/";

struct OpenApiExtension;

impl zed::Extension for OpenApiExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<zed::Command> {
        let command = self.server_path(language_server_id, worktree)?;
        Ok(zed::Command {
            command,
            args: Vec::new(),
            env: Vec::new(),
        })
    }

    fn language_server_initialization_options(
        &mut self,
        server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<Option<serde_json::Value>> {
        Ok(LspSettings::for_worktree(server_id.as_ref(), worktree)
            .ok()
            .and_then(|settings| settings.initialization_options))
    }

    fn language_server_workspace_configuration(
        &mut self,
        server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<Option<serde_json::Value>> {
        Ok(LspSettings::for_worktree(server_id.as_ref(), worktree)
            .ok()
            .and_then(|settings| settings.settings))
    }
}

impl OpenApiExtension {
    /// A `PATH` binary if there is one, else a downloaded release binary.
    fn server_path(
        &self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<String> {
        if let Some(path) = worktree.which(SERVER_BINARY) {
            return Ok(path);
        }
        self.download_server(language_server_id).map_err(|err| {
            format!(
                "{err}\n\nAlternatively, put {SERVER_BINARY} on PATH — from a \
                 checkout of {REPO}, run: cargo install --path openapi-lsp"
            )
        })
    }

    fn download_server(&self, language_server_id: &LanguageServerId) -> Result<String> {
        let (os, arch) = zed::current_platform();
        let target = platform_target(os, arch)?;

        zed::set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::CheckingForUpdate,
        );
        let release = zed::latest_github_release(
            REPO,
            GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;

        // Asset names carry only the platform — the release itself carries the
        // version — so the tag is needed just to key the install directory.
        // Tags read `lsp-v<x.y.z>`; drop that prefix for a tidier name.
        let version = release.version.strip_prefix("lsp-").unwrap_or(&release.version);
        let stem = format!("openapi-lsp-{target}");
        let (asset_name, file_type, binary_name) = if matches!(os, Os::Windows) {
            (
                format!("{stem}.zip"),
                DownloadedFileType::Zip,
                "openapi-lsp.exe",
            )
        } else {
            (
                format!("{stem}.tar.gz"),
                DownloadedFileType::GzipTar,
                "openapi-lsp",
            )
        };

        // Archives hold their files at the root, so the binary lands straight
        // in the install directory. That directory is keyed by version, which
        // is what makes an upgrade a fresh download rather than an overwrite.
        let install_dir = format!("openapi-lsp-{version}");
        let binary_path = format!("{install_dir}/{binary_name}");

        if !std::fs::metadata(&binary_path).is_ok_and(|stat| stat.is_file()) {
            let asset = release
                .assets
                .iter()
                .find(|asset| asset.name == asset_name)
                .ok_or_else(|| {
                    format!("release {} has no asset '{asset_name}'", release.version)
                })?;

            if !asset.download_url.starts_with(GITHUB_RELEASE_PREFIX) {
                return Err(format!(
                    "unexpected download URL for '{asset_name}': {}",
                    asset.download_url
                ));
            }

            zed::set_language_server_installation_status(
                language_server_id,
                &LanguageServerInstallationStatus::Downloading,
            );
            zed::download_file(&asset.download_url, &install_dir, file_type)
                .map_err(|err| format!("failed to download {asset_name}: {err}"))?;

            zed::make_file_executable(&binary_path)
                .map_err(|err| format!("failed to make {binary_name} executable: {err}"))?;
        }

        remove_other_versions(&install_dir);
        Ok(binary_path)
    }
}

/// Drop the directories left behind by earlier versions. Best-effort: a
/// directory we cannot remove only costs disk space.
fn remove_other_versions(install_dir: &str) {
    let Ok(entries) = std::fs::read_dir(".") else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("openapi-lsp-") && name != install_dir {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// The release target triple for this platform, matching the names the
/// release workflow builds.
fn platform_target(os: Os, arch: Architecture) -> Result<&'static str> {
    match (os, arch) {
        (Os::Linux, Architecture::X8664) => Ok("x86_64-unknown-linux-gnu"),
        (Os::Linux, Architecture::Aarch64) => Ok("aarch64-unknown-linux-gnu"),
        (Os::Mac, Architecture::X8664) => Ok("x86_64-apple-darwin"),
        (Os::Mac, Architecture::Aarch64) => Ok("aarch64-apple-darwin"),
        (Os::Windows, Architecture::X8664) => Ok("x86_64-pc-windows-msvc"),
        _ => Err(format!(
            "no {SERVER_BINARY} release is built for ({os:?}, {arch:?}); \
             build it from source instead"
        )),
    }
}

zed::register_extension!(OpenApiExtension);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_target_is_reachable() {
        for (os, arch, expected) in [
            (Os::Linux, Architecture::X8664, "x86_64-unknown-linux-gnu"),
            (Os::Linux, Architecture::Aarch64, "aarch64-unknown-linux-gnu"),
            (Os::Mac, Architecture::X8664, "x86_64-apple-darwin"),
            (Os::Mac, Architecture::Aarch64, "aarch64-apple-darwin"),
            (Os::Windows, Architecture::X8664, "x86_64-pc-windows-msvc"),
        ] {
            assert_eq!(platform_target(os, arch).unwrap(), expected);
        }
    }

    #[test]
    fn an_unbuilt_target_says_which_one() {
        let err = platform_target(Os::Windows, Architecture::Aarch64).unwrap_err();
        assert!(err.contains("Aarch64") && err.contains("Windows"), "{err}");
    }
}
