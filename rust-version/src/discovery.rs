//! Model-discovery helpers mirroring ``llama_launcher/discovery.py``.
//!
//! Recursively scans directories for ``.gguf`` files, returning a sorted,
//! deduplicated list.

use std::path::Path;

/// Recursively find ``.gguf`` files in *model_dirs*.
///
/// Returns a sorted, deduplicated list of path strings (case-insensitive sort,
/// matching legacy ``sorted(set(models), key=str.lower)``).
pub fn scan_gguf_models(model_dirs: &[String]) -> Vec<String> {
    let mut models: Vec<String> = Vec::new();

    for folder in model_dirs {
        let d = Path::new(folder);
        if d.exists() && d.is_dir() {
            if let Ok(entries) = walk_dir_gguf(d) {
                models.extend(entries);
            }
        }
    }

    // Deduplicate
    models.sort();
    models.dedup();

    // Case-insensitive sort (mirrors legacy key=str.lower)
    models.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));

    models
}

/// Recursively walk *dir* collecting paths ending with ``.gguf``.
fn walk_dir_gguf(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut results = Vec::new();
    collect_gguf(dir, &mut results)?;
    Ok(results)
}

fn collect_gguf(dir: &Path, results: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_gguf(&path, results).ok();
        } else if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext.eq_ignore_ascii_case("gguf") {
                    results.push(path.to_string_lossy().to_string());
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Acceptance: scan_gguf_models returns sorted, deduplicated list.
    #[test]
    fn test_scan_gguf_models_basic() {
        let tmp = TempDir::new().expect("create temp dir");
        let base = tmp.path();

        // Create nested structure:
        //   models/
        //     A/model1.gguf
        //     B/model2.gguf
        //     B/model3.GGUF   (upper-case extension)
        //   other/
        //     not_a_model.bin
        fs::create_dir_all(base.join("models/A")).unwrap();
        fs::create_dir_all(base.join("models/B")).unwrap();
        fs::create_dir_all(base.join("other")).unwrap();

        fs::write(base.join("models/A/model1.gguf"), "").unwrap();
        fs::write(base.join("models/B/model2.gguf"), "").unwrap();
        fs::write(base.join("models/B/model3.GGUF"), "").unwrap();
        fs::write(base.join("other/not_a_model.bin"), "").unwrap();

        let dirs = vec![base.join("models").to_string_lossy().to_string()];
        let result = scan_gguf_models(&dirs);

        assert_eq!(result.len(), 3);
        // All three .gguf files found (case-insensitive extension match)
        assert!(result.iter().any(|p| p.contains("model1.gguf")));
        assert!(result.iter().any(|p| p.contains("model2.gguf")));
        assert!(result.iter().any(|p| p.contains("model3.GGUF")));
        // .bin file excluded
        assert!(!result.iter().any(|p| p.contains(".bin")));
    }

    /// Acceptance: duplicate paths are deduplicated.
    #[test]
    fn test_scan_gguf_models_dedup() {
        let tmp = TempDir::new().expect("create temp dir");
        let base = tmp.path();

        fs::create_dir_all(base).unwrap();
        fs::write(base.join("dup.gguf"), "").unwrap();

        // Same directory listed twice
        let dir = base.to_string_lossy().to_string();
        let dirs = vec![dir.clone(), dir.clone()];
        let result = scan_gguf_models(&dirs);

        assert_eq!(result.len(), 1);
        assert!(result[0].contains("dup.gguf"));
    }

    /// Acceptance: nonexistent directories are silently skipped.
    #[test]
    fn test_scan_gguf_models_missing_dir() {
        let dirs = vec![
            "/nonexistent/path/that/does/not/exist".into(),
        ];
        let result = scan_gguf_models(&dirs);
        assert!(result.is_empty());
    }

    /// Acceptance: empty model_dirs returns empty list.
    #[test]
    fn test_scan_gguf_models_empty_dirs() {
        let result = scan_gguf_models(&[]);
        assert!(result.is_empty());
    }

    /// Acceptance: results are sorted case-insensitively.
    #[test]
    fn test_scan_gguf_models_case_insensitive_sort() {
        let tmp = TempDir::new().expect("create temp dir");
        let base = tmp.path();

        // All files in the same directory so sort is purely on filenames
        fs::create_dir_all(base).unwrap();

        // Files that would sort differently case-sensitively vs case-insensitively
        fs::write(base.join("Zebra.gguf"), "").unwrap();
        fs::write(base.join("alpha.gguf"), "").unwrap();
        fs::write(base.join("Middle.gguf"), "").unwrap();

        let dirs = vec![base.to_string_lossy().to_string()];
        let result = scan_gguf_models(&dirs);

        assert_eq!(result.len(), 3);
        // Case-insensitive order: alpha < Middle < Zebra
        assert!(result[0].to_lowercase().contains("alpha"));
        assert!(result[1].to_lowercase().contains("middle"));
        assert!(result[2].to_lowercase().contains("zebra"));
    }
}
