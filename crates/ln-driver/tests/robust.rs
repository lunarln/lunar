//! Editors analyze half-typed code with `keep_going`; the compiler must not
//! panic on any prefix (or small mutilation) of a valid or invalid program.

use ln_driver::{AnalyzeOptions, analyze_with, path_key};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn sources() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = Vec::new();
    for dir in ["examples", "tests/run", "tests/ui"] {
        for e in std::fs::read_dir(root.join(dir)).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "ln") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Analyze `text` as the contents of `path`; returns a panic message, if any.
fn try_analyze(path: &Path, text: &str) -> Option<String> {
    let mut overlay = HashMap::new();
    overlay.insert(path_key(path), text.to_string());
    let opts = AnalyzeOptions { keep_going: true, overlay: Some(&overlay), ..Default::default() };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let a = analyze_with(path, &opts);
        // Exercise the editor queries too.
        if let Some(file) = ln_driver::ide::file_for_path(&a.map, path) {
            let len = a.map.get(file).text.len() as u32;
            for off in (0..len).step_by(7) {
                let _ = ln_driver::ide::hover(&a, file, off);
                let _ = ln_driver::ide::definition(&a, file, off);
            }
        }
    }));
    r.err().map(|e| e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default())
}

fn run(cuts_per_file: usize) {
    std::panic::set_hook(Box::new(|_| {}));
    let mut failures = Vec::new();
    for path in sources() {
        let text = std::fs::read_to_string(&path).unwrap();
        let bounds: Vec<usize> = (0..=text.len()).filter(|i| text.is_char_boundary(*i)).collect();
        let step = (bounds.len() / cuts_per_file).max(1);
        for &cut in bounds.iter().step_by(step) {
            let mut variants = vec![text[..cut].to_string()];
            // Also delete a few characters in the middle instead of truncating.
            if let Some(&end) = bounds.iter().find(|&&b| b >= cut + 3) {
                variants.push(format!("{}{}", &text[..cut], &text[end..]));
            }
            for v in variants {
                if let Some(msg) = try_analyze(&path, &v) {
                    failures.push(format!("{} (cut at byte {cut}): {msg}", path.display()));
                    break;
                }
            }
        }
    }
    let _ = std::panic::take_hook();
    assert!(failures.is_empty(), "{} panics:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn analysis_never_panics_on_broken_code() {
    run(12);
}

/// The thorough version: `cargo test --release -p ln-driver --test robust -- --ignored`.
#[test]
#[ignore]
fn analysis_never_panics_on_broken_code_thorough() {
    run(400);
}
