//! Recursive resolution of local OpenQASM include files.
//!
//! The standard-library include (`stdgates.inc`) remains an include directive
//! because its gate signatures are built into semantic analysis. Other include
//! paths are resolved relative to the file that contains the directive and
//! expanded before parsing.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::lexer::{self, Token};

#[derive(Debug)]
pub enum IncludeError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    InvalidDirective {
        path: PathBuf,
        offset: usize,
    },
    Cycle {
        path: PathBuf,
    },
}

impl std::fmt::Display for IncludeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IncludeError::Io { path, source } => {
                write!(f, "failed to read include {}: {}", path.display(), source)
            }
            IncludeError::InvalidDirective { path, offset } => write!(
                f,
                "invalid include directive in {} at byte {}",
                path.display(),
                offset
            ),
            IncludeError::Cycle { path } => {
                write!(f, "include cycle detected at {}", path.display())
            }
        }
    }
}

impl std::error::Error for IncludeError {}

pub fn load_with_includes(path: impl AsRef<Path>) -> Result<String, IncludeError> {
    let path = path.as_ref();
    let mut active = HashSet::new();
    load_recursive(path, &mut active, true)
}

fn load_recursive(
    path: &Path,
    active: &mut HashSet<PathBuf>,
    keep_header: bool,
) -> Result<String, IncludeError> {
    let canonical = path.canonicalize().map_err(|source| IncludeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !active.insert(canonical.clone()) {
        return Err(IncludeError::Cycle { path: canonical });
    }

    let source = fs::read_to_string(&canonical).map_err(|source| IncludeError::Io {
        path: canonical.clone(),
        source,
    })?;
    let base = canonical.parent().unwrap_or_else(|| Path::new("."));
    let expanded = expand_source(&source, &canonical, base, active, keep_header)?;
    active.remove(&canonical);
    Ok(expanded)
}

fn expand_source(
    source: &str,
    source_path: &Path,
    base: &Path,
    active: &mut HashSet<PathBuf>,
    keep_header: bool,
) -> Result<String, IncludeError> {
    let (tokens, lex_errors) = lexer::lex(source);
    if let Some(span) = lex_errors.first() {
        return Err(IncludeError::InvalidDirective {
            path: source_path.to_path_buf(),
            offset: span.start,
        });
    }

    let mut replacements = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if matches!(tokens[index].node, Token::Include) {
            let Some(path_token) = tokens.get(index + 1) else {
                return Err(IncludeError::InvalidDirective {
                    path: source_path.to_path_buf(),
                    offset: tokens[index].span.start,
                });
            };
            let Some(end_token) = tokens.get(index + 2) else {
                return Err(IncludeError::InvalidDirective {
                    path: source_path.to_path_buf(),
                    offset: tokens[index].span.start,
                });
            };
            let Token::StringLiteral(include_path) = &path_token.node else {
                return Err(IncludeError::InvalidDirective {
                    path: source_path.to_path_buf(),
                    offset: tokens[index].span.start,
                });
            };
            if !matches!(end_token.node, Token::Semicolon) {
                return Err(IncludeError::InvalidDirective {
                    path: source_path.to_path_buf(),
                    offset: tokens[index].span.start,
                });
            }

            if include_path != "stdgates.inc" {
                let included = load_recursive(&base.join(include_path), active, false)?;
                replacements.push((
                    tokens[index].span.start..end_token.span.end,
                    format!("// expanded {}\n{}", include_path, included),
                ));
            }
            index += 3;
            continue;
        }
        index += 1;
    }

    let mut expanded = source.to_string();
    for (span, replacement) in replacements.into_iter().rev() {
        expanded.replace_range(span, &replacement);
    }
    if !keep_header {
        expanded = strip_version_header(&expanded);
    }
    Ok(expanded)
}

fn strip_version_header(source: &str) -> String {
    let (tokens, _) = lexer::lex(source);
    if matches!(
        tokens.first().map(|token| &token.node),
        Some(Token::OpenQasm)
    ) {
        if let Some(end) = tokens
            .iter()
            .find(|token| matches!(token.node, Token::Semicolon))
            .map(|token| token.span.end)
        {
            return source[end..].to_string();
        }
    }
    source.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("qasm-rs-includes-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn expands_nested_local_includes() {
        let dir = temp_dir();
        fs::write(dir.join("gates.inc"), "gate custom q { x q; }\n").unwrap();
        fs::write(
            dir.join("main.qasm"),
            "OPENQASM 3.0; include \"gates.inc\"; qubit q; custom q;",
        )
        .unwrap();

        let expanded = load_with_includes(dir.join("main.qasm")).unwrap();
        fs::remove_dir_all(&dir).unwrap();
        assert!(expanded.contains("gate custom"));
        assert!(!expanded.contains("include \"gates.inc\""));
    }

    #[test]
    fn rejects_include_cycles() {
        let dir = temp_dir();
        fs::write(dir.join("a.qasm"), "OPENQASM 3.0; include \"b.qasm\";").unwrap();
        fs::write(dir.join("b.qasm"), "OPENQASM 3.0; include \"a.qasm\";").unwrap();

        let error = load_with_includes(dir.join("a.qasm")).unwrap_err();
        fs::remove_dir_all(&dir).unwrap();
        assert!(matches!(error, IncludeError::Cycle { .. }));
    }
}
