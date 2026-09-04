use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use regex::Regex;

const DEFAULT_SOURCES: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    "SOGNI.md",
    "STYLE.md",
    "ARTIST.md",
    "BRAND.md",
    "sogni.md",
    ".sogni/*.md",
    ".sogni/**/*.md",
];
const SKIPPED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "dist",
    "dist-esm",
    "build",
    "coverage",
    ".next",
    ".turbo",
    "output",
    "target",
];

#[derive(Clone, Debug)]
pub struct ContextOptions {
    pub workspace: PathBuf,
    pub sources: Vec<String>,
    pub auto: bool,
    pub max_total_chars: usize,
    pub max_file_chars: usize,
}

#[derive(Clone, Debug)]
pub struct ContextDoc {
    pub relative_path: String,
    pub original_chars: usize,
    pub included_chars: usize,
    pub truncated_by_file: bool,
    pub truncated_by_total: bool,
    pub content: String,
}

#[derive(Clone, Debug, Default)]
pub struct LoadedContext {
    pub sources: Vec<String>,
    pub docs: Vec<ContextDoc>,
    pub warnings: Vec<String>,
    pub total_chars: usize,
    pub budget_exhausted: bool,
}

pub fn load(options: &ContextOptions) -> LoadedContext {
    let records = DEFAULT_SOURCES
        .iter()
        .filter(|_| options.auto)
        .map(|source| ((*source).to_owned(), true))
        .chain(
            options
                .sources
                .iter()
                .cloned()
                .map(|source| (source, false)),
        )
        .collect::<Vec<_>>();
    let mut context = LoadedContext {
        sources: records.iter().map(|(source, _)| source.clone()).collect(),
        ..LoadedContext::default()
    };
    let mut seen = HashSet::new();
    let mut files = Vec::new();
    for (source, automatic) in records {
        let (expanded, warnings) = expand_source(&source, &options.workspace);
        context.warnings.extend(
            warnings
                .into_iter()
                .filter(|warning| !automatic || !warning.starts_with("Context source not found:")),
        );
        for file in expanded {
            let canonical = fs::canonicalize(&file).unwrap_or(file);
            if seen.insert(canonical.clone()) {
                files.push(canonical);
            }
        }
    }
    files.sort_by_key(|path| relative(&options.workspace, path));
    for path in files {
        if context.total_chars >= options.max_total_chars {
            context.budget_exhausted = true;
            break;
        }
        let mut content = match fs::read_to_string(&path) {
            Ok(content) => content.replace('\0', ""),
            Err(error) => {
                context
                    .warnings
                    .push(format!("Could not read {}: {error}", path.display()));
                continue;
            }
        };
        let original_chars = content.chars().count();
        let mut truncated_by_file = false;
        if original_chars > options.max_file_chars {
            content = truncate_chars(&content, options.max_file_chars);
            content.push_str("\n\n[Truncated: per-file context budget exceeded.]");
            truncated_by_file = true;
        }
        let remaining = options.max_total_chars.saturating_sub(context.total_chars);
        let mut truncated_by_total = false;
        if content.chars().count() > remaining {
            content = truncate_chars(&content, remaining);
            truncated_by_total = true;
            context.budget_exhausted = true;
        }
        let included_chars = content.chars().count();
        context.total_chars += included_chars;
        context.docs.push(ContextDoc {
            relative_path: relative(&options.workspace, &path),
            original_chars,
            included_chars,
            truncated_by_file,
            truncated_by_total,
            content,
        });
    }
    context
}

fn expand_source(source: &str, workspace: &Path) -> (Vec<PathBuf>, Vec<String>) {
    let expanded = expand_home(source);
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        workspace.join(expanded)
    };
    if has_glob(source) {
        let pattern = portable(&absolute);
        let Some(base) = glob_base(&absolute) else {
            return (
                Vec::new(),
                vec![format!("Context source not found: {source}")],
            );
        };
        if !base.exists() {
            return (
                Vec::new(),
                vec![format!("Context source not found: {}", base.display())],
            );
        }
        let mut warnings = Vec::new();
        let files = walk_markdown(&base, 0, &mut warnings)
            .into_iter()
            .filter(|path| glob_regex(&pattern).is_ok_and(|regex| regex.is_match(&portable(path))))
            .collect();
        return (files, warnings);
    }
    if !absolute.exists() {
        return (
            Vec::new(),
            vec![format!("Context source not found: {}", absolute.display())],
        );
    }
    if absolute.is_dir() {
        let mut warnings = Vec::new();
        let files = walk_markdown(&absolute, 0, &mut warnings);
        if files.is_empty() {
            warnings.push(format!("No Markdown files found in {}", absolute.display()));
        }
        return (files, warnings);
    }
    if absolute.is_file() && is_markdown(&absolute) {
        (vec![absolute], Vec::new())
    } else {
        (
            Vec::new(),
            vec![format!(
                "Skipping non-Markdown context source: {}",
                absolute.display()
            )],
        )
    }
}

fn walk_markdown(root: &Path, depth: usize, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    if depth > 20 {
        return Vec::new();
    }
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => {
            warnings.push(format!(
                "Could not read directory {}: {error}",
                root.display()
            ));
            return Vec::new();
        }
    };
    let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    let mut files = Vec::new();
    for entry in entries {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        // Never traverse symlinks: a workspace context glob must not silently
        // escape into unrelated or sensitive directories.
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if !SKIPPED_DIRS.contains(&name.as_str()) {
                files.extend(walk_markdown(&path, depth + 1, warnings));
            }
        } else if kind.is_file() && is_markdown(&path) {
            files.push(path);
        }
    }
    files
}

fn glob_regex(pattern: &str) -> Result<Regex, regex::Error> {
    let mut output = String::from("^");
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        match (chars[index], chars.get(index + 1), chars.get(index + 2)) {
            ('*', Some('*'), Some('/')) => {
                output.push_str("(?:.*/)?");
                index += 3;
            }
            ('*', Some('*'), _) => {
                output.push_str(".*");
                index += 2;
            }
            ('*', _, _) => {
                output.push_str("[^/]*");
                index += 1;
            }
            ('?', _, _) => {
                output.push_str("[^/]");
                index += 1;
            }
            (character, _, _) => {
                output.push_str(&regex::escape(&character.to_string()));
                index += 1;
            }
        }
    }
    output.push('$');
    Regex::new(&output)
}

fn glob_base(pattern: &Path) -> Option<PathBuf> {
    let mut base = PathBuf::new();
    for component in pattern.components() {
        let part = component.as_os_str().to_string_lossy();
        if has_glob(&part) {
            break;
        }
        base.push(component.as_os_str());
    }
    (!base.as_os_str().is_empty()).then_some(base)
}

fn expand_home(source: &str) -> PathBuf {
    if source == "~" {
        return std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(source));
    }
    if let Some(rest) = source
        .strip_prefix("~/")
        .or_else(|| source.strip_prefix("~\\"))
    {
        if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(source)
}

fn relative(workspace: &Path, path: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn portable(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
fn has_glob(value: &str) -> bool {
    value.contains(['*', '?', '[', ']', '{', '}'])
}
fn is_markdown(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}
fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_star_matches_zero_or_more_directories() {
        let regex = glob_regex("C:/repo/.sogni/**/*.md").unwrap();
        assert!(regex.is_match("C:/repo/.sogni/style.md"));
        assert!(regex.is_match("C:/repo/.sogni/client/style.md"));
        assert!(!regex.is_match("C:/repo/.sogni/style.txt"));
    }
}
