//! Spec files a `$ref` in one document could point at.
//!
//! Completion needs candidate paths *before* anything is written, so this
//! looks at the filesystem rather than at the open documents. The scan starts
//! `scan_up` directories above the document and descends `scan_down` levels
//! below the document's own directory, which covers the usual
//! `paths/*.yaml` + `components/**/*.yaml` split. Only files sharing the
//! document's extension are offered unless the settings name others.
//!
//! The editor's workspace folders are a hard ceiling: whatever `scan_up`
//! says, we never look outside the directory the user actually opened.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use crate::config::RefFileSettings;
use crate::document::uri_to_path;

/// Directories visited per scan. Bounds the walk itself, which `max_files`
/// cannot: a deep tree may hold thousands of directories and no spec at all.
const MAX_DIRS: usize = 2_000;
/// Files whose contents we remember before starting over.
const MAX_CACHED_FILES: usize = 2_000;
/// How long a scan is reused. Long enough to cover a burst of keystrokes,
/// short enough that a file added in the editor shows up promptly.
const CACHE_TTL: Duration = Duration::from_secs(2);

/// A file a `$ref` could name.
#[derive(Clone, Debug)]
pub struct Candidate {
    /// Written the way a `$ref` writes it: `./pet.yaml`, `../shared/pet.yaml`.
    pub relative: String,
    pub path: PathBuf,
}

/// The spec files near the document at `from_uri`, nearest first.
pub fn ref_candidates(
    from_uri: &str,
    settings: &RefFileSettings,
    roots: &[PathBuf],
) -> Vec<Candidate> {
    let Some(path) = uri_to_path(from_uri) else {
        return Vec::new();
    };
    let Some(dir) = path.parent() else {
        return Vec::new();
    };

    let extensions = if settings.extensions.is_empty() {
        // Same suffix as the document being edited.
        match path.extension().and_then(|ext| ext.to_str()) {
            Some(ext) => vec![ext.to_ascii_lowercase()],
            None => return Vec::new(),
        }
    } else {
        settings
            .extensions
            .iter()
            .map(|ext| ext.trim_start_matches('.').to_ascii_lowercase())
            .collect()
    };

    // Ascending fewer levels than asked for is fine — we hit the ceiling of
    // what the user opened, or ran out of tree.
    let ceiling = ceiling(dir, roots);
    let mut root = dir;
    let mut climbed = 0;
    while climbed < settings.scan_up && Some(root) != ceiling.as_deref() {
        let Some(parent) = root.parent() else { break };
        root = parent;
        climbed += 1;
    }
    // Depth is measured from `root`, so the document's own subtree still gets
    // `scan_down` levels once the climb is accounted for.
    let max_depth = climbed + settings.scan_down;

    let found = cached_scan(root, &extensions, max_depth, settings);
    let mut out: Vec<Candidate> = found
        .iter()
        .filter(|file| *file != &path)
        .filter_map(|file| {
            Some(Candidate {
                relative: relative(dir, file)?,
                path: file.clone(),
            })
        })
        .collect();
    out.sort_by(|a, b| {
        // Same-directory files first: they are what people reference most.
        let (a, b) = (&a.relative, &b.relative);
        a.starts_with("..")
            .cmp(&b.starts_with(".."))
            .then_with(|| a.len().cmp(&b.len()))
            .then_with(|| a.cmp(b))
    });
    out.dedup_by(|a, b| a.relative == b.relative);
    out
}

/// Names of the entries `path` holds at `pointer`, e.g. the schema names under
/// `/components/schemas`. Empty when the file has no such node, does not
/// parse, or cannot be read.
///
/// Results are cached until the file's size or modification time changes: one
/// completion request asks about every neighbouring file, and the answer only
/// moves when the file does.
pub fn component_names(path: &Path, pointer: &str) -> Vec<String> {
    static CACHE: OnceLock<Mutex<HashMap<NamesKey, NamesEntry>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let stamp = stamp(path);

    let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    let key = (path.to_path_buf(), pointer.to_string());
    if let Some((cached, names)) = cache.get(&key)
        && *cached == stamp
    {
        return names.clone();
    }

    let names = read_component_names(path, pointer);
    // A session may touch many files; forget everything rather than grow
    // without bound, since a rebuild costs one parse per file asked about.
    if cache.len() > MAX_CACHED_FILES {
        cache.clear();
    }
    cache.insert(key, (stamp, names.clone()));
    names
}

/// A file and the pointer looked up inside it.
type NamesKey = (PathBuf, String);
/// The names found, and the file state they were read from.
type NamesEntry = (Stamp, Vec<String>);
/// Cheap change detector: modification time and size, or `None` for a file we
/// could not stat, which then simply never caches as unchanged.
type Stamp = Option<(std::time::SystemTime, u64)>;

fn stamp(path: &Path) -> Stamp {
    let meta = fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

fn read_component_names(path: &Path, pointer: &str) -> Vec<String> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = yaml_serde::from_str::<yaml_serde::Value>(&text) else {
        return Vec::new();
    };
    crate::pointer::get_at_pointer(&value, pointer)
        .map(crate::pointer::mapping_keys)
        .unwrap_or_default()
}

/// Markers of a checkout root, used only when the editor told us nothing
/// about its workspace folders.
const PROJECT_MARKERS: [&str; 2] = [".git", ".hg"];

/// The highest directory the scan may reach: the innermost workspace folder
/// holding `dir`, or — with no usable folder — the checkout `dir` sits in.
fn ceiling(dir: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    let folder = roots
        .iter()
        .filter(|root| dir.starts_with(root))
        // Innermost wins, so a nested folder is not widened by its parent.
        .max_by_key(|root| root.components().count());
    if let Some(folder) = folder {
        return Some(folder.clone());
    }
    let mut candidate = Some(dir);
    while let Some(current) = candidate {
        if PROJECT_MARKERS
            .iter()
            .any(|marker| current.join(marker).exists())
        {
            return Some(current.to_path_buf());
        }
        candidate = current.parent();
    }
    None
}

/// Key of a scan: the same root and filters always yield the same file list,
/// whichever document in the tree asked for it.
type CacheKey = (PathBuf, Vec<String>, usize);
/// A scan result and when it was taken.
type CacheEntry = (Instant, Vec<PathBuf>);

fn cached_scan(
    root: &Path,
    extensions: &[String],
    max_depth: usize,
    settings: &RefFileSettings,
) -> Vec<PathBuf> {
    static CACHE: OnceLock<Mutex<HashMap<CacheKey, CacheEntry>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = (root.to_path_buf(), extensions.to_vec(), max_depth);

    let mut cache = cache.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((at, files)) = cache.get(&key)
        && at.elapsed() < CACHE_TTL
    {
        return files.clone();
    }

    let mut scan = Scan {
        extensions,
        skip_dirs: &settings.skip_dirs,
        max_files: settings.max_files,
        dirs_left: MAX_DIRS,
        files: Vec::new(),
    };
    scan.walk(root, 0, max_depth);
    let files = scan.files;

    // Drop entries nobody has asked about for a while, so the map cannot grow
    // without bound over a long session.
    cache.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
    cache.insert(key, (Instant::now(), files.clone()));
    files
}

struct Scan<'a> {
    extensions: &'a [String],
    skip_dirs: &'a [String],
    max_files: usize,
    dirs_left: usize,
    files: Vec<PathBuf>,
}

impl Scan<'_> {
    fn walk(&mut self, dir: &Path, depth: usize, max_depth: usize) {
        if depth > max_depth || self.files.len() >= self.max_files || self.dirs_left == 0 {
            return;
        }
        self.dirs_left -= 1;
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            if self.files.len() >= self.max_files {
                return;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') || self.skip_dirs.iter().any(|skip| skip == name) {
                continue;
            }
            // Symlinks are skipped rather than followed: they can point
            // outside the tree, or back into it in a cycle.
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                subdirs.push(entry.path());
            } else if self.matches(&entry.path()) {
                self.files.push(entry.path());
            }
        }
        for subdir in subdirs {
            self.walk(&subdir, depth + 1, max_depth);
        }
    }

    fn matches(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| self.extensions.contains(&ext.to_ascii_lowercase()))
    }
}

/// `file` expressed relative to `from`, e.g. `./pet.yaml` or `../shared/pet.yaml`.
fn relative(from: &Path, file: &Path) -> Option<String> {
    let from: Vec<_> = from.components().collect();
    let to: Vec<_> = file.components().collect();
    let shared = from.iter().zip(&to).take_while(|(a, b)| a == b).count();

    let mut parts: Vec<String> = vec!["..".to_string(); from.len() - shared];
    parts.extend(
        to[shared..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    if parts.is_empty() {
        return None;
    }
    let joined = parts.join("/");
    Some(if joined.starts_with("..") {
        joined
    } else {
        // Explicitly relative, so the path never reads as a URI scheme.
        format!("./{joined}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_point_up_and_down() {
        assert_eq!(
            relative(Path::new("/a/b"), Path::new("/a/b/pet.yaml")).as_deref(),
            Some("./pet.yaml")
        );
        assert_eq!(
            relative(Path::new("/a/b"), Path::new("/a/c/pet.yaml")).as_deref(),
            Some("../c/pet.yaml")
        );
        assert_eq!(
            relative(Path::new("/a/b"), Path::new("/a/b/sub/pet.yaml")).as_deref(),
            Some("./sub/pet.yaml")
        );
    }

    /// A tree shaped like a split spec: the document sits in `api/paths/`,
    /// its schemas live in `api/components/`, and there is noise around.
    fn tree() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "openapi-ls-files-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("api/paths/nested")).unwrap();
        fs::create_dir_all(root.join("api/components")).unwrap();
        fs::create_dir_all(root.join("node_modules")).unwrap();
        fs::write(root.join("api/paths/pets.yaml"), "get: {}\n").unwrap();
        fs::write(root.join("api/paths/nested/tags.yaml"), "get: {}\n").unwrap();
        fs::write(
            root.join("api/components/pet.yaml"),
            "components:\n  schemas:\n    Pet: {}\n    Tag: {}\n",
        )
        .unwrap();
        fs::write(root.join("api/components/pet.json"), "{}\n").unwrap();
        fs::write(root.join("api/paths/notes.md"), "hi\n").unwrap();
        fs::write(root.join("node_modules/junk.yaml"), "a: 1\n").unwrap();
        root
    }

    fn candidates(root: &Path, settings: &RefFileSettings) -> Vec<String> {
        let uri = crate::document::path_to_uri(&root.join("api/paths/pets.yaml"));
        // Each test builds a fresh tree, so the scan cache cannot collide.
        ref_candidates(&uri, settings, &[root.to_path_buf()])
            .into_iter()
            .map(|candidate| candidate.relative)
            .collect()
    }

    #[test]
    fn offers_neighbours_of_the_same_suffix() {
        let root = tree();
        let found = candidates(&root, &RefFileSettings::default());

        assert!(found.contains(&"../components/pet.yaml".into()), "{found:?}");
        assert!(found.contains(&"./nested/tags.yaml".into()), "{found:?}");
        // Itself, other suffixes, markdown and skipped directories stay out.
        assert!(!found.contains(&"./pets.yaml".into()), "{found:?}");
        assert!(!found.iter().any(|f| f.ends_with(".json")), "{found:?}");
        assert!(!found.iter().any(|f| f.ends_with(".md")), "{found:?}");
        assert!(!found.iter().any(|f| f.contains("node_modules")), "{found:?}");
        // Same directory before anything reached through `..`.
        let first_up = found.iter().position(|f| f.starts_with("..")).unwrap();
        let last_here = found.iter().rposition(|f| f.starts_with("./")).unwrap();
        assert!(last_here < first_up, "{found:?}");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn component_names_come_from_the_file_and_survive_a_rewrite() {
        let root = tree();
        let pet = root.join("api/components/pet.yaml");
        assert_eq!(
            component_names(&pet, "/components/schemas"),
            vec!["Pet".to_string(), "Tag".to_string()]
        );
        // No such node, and no such file: both quietly offer nothing.
        assert!(component_names(&pet, "/components/responses").is_empty());
        assert!(component_names(&root.join("nope.yaml"), "/components/schemas").is_empty());

        fs::write(&pet, "components:\n  schemas:\n    Pet: {}\n").unwrap();
        assert_eq!(
            component_names(&pet, "/components/schemas"),
            vec!["Pet".to_string()],
            "the cache should have noticed the rewrite"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn settings_bound_the_scan() {
        let root = tree();

        // Without climbing, the sibling directory is out of reach.
        let here = candidates(
            &root,
            &RefFileSettings {
                scan_up: 0,
                ..Default::default()
            },
        );
        assert!(here.contains(&"./nested/tags.yaml".into()), "{here:?}");
        assert!(!here.iter().any(|f| f.starts_with("..")), "{here:?}");

        // Without descending, the nested file is out of reach.
        let flat = candidates(
            &root,
            &RefFileSettings {
                scan_down: 0,
                ..Default::default()
            },
        );
        assert!(flat.contains(&"../components/pet.yaml".into()), "{flat:?}");
        assert!(!flat.contains(&"./nested/tags.yaml".into()), "{flat:?}");

        // Explicit extensions replace "same suffix as the document".
        let json = candidates(
            &root,
            &RefFileSettings {
                extensions: vec![".json".into()],
                ..Default::default()
            },
        );
        assert_eq!(json, vec!["../components/pet.json".to_string()]);

        let _ = fs::remove_dir_all(&root);
    }
}
