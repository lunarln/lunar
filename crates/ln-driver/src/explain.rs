//! `lunar explain`: long-form explanations of diagnostic codes, embedded from
//! `docs/errors/<scope>/<name>.md`.

use ln_diag::{ALL_CODES, Diagnostic};
use std::process::ExitCode;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/explain_files.rs"));
}

/// The Markdown explanation of `code` (`scope::name`), if there is one.
pub fn lookup(code: &str) -> Option<&'static str> {
    embedded::EXPLANATIONS.iter().find(|(c, _)| *c == code).map(|(_, md)| *md)
}

/// The one-line title of an explanation: its first line, minus `# code: `.
fn title(md: &'static str) -> &'static str {
    let first = md.lines().next().unwrap_or("").trim_start_matches('#').trim();
    first.split_once(": ").map_or(first, |(_, t)| t)
}

/// Accept `scope::name` as well as pasted `err(scope::name)` / `warn(...)`.
fn normalize(input: &str) -> String {
    let mut s = input.trim().trim_matches('`').trim_end_matches(':');
    for prefix in ["err(", "warn("] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.strip_suffix(')').unwrap_or(rest);
        }
    }
    s.trim().to_string()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + usize::from(ca != *cb)).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Known codes closest to `code`, comparing both the full code and just the
/// name (so a missing or wrong scope still finds a match).
fn suggestions(code: &str) -> Vec<String> {
    let name = code.rsplit("::").next().unwrap_or(code);
    let scored: Vec<(usize, String)> = ALL_CODES
        .iter()
        .map(|c| {
            let full = c.to_string();
            (edit_distance(code, &full).min(edit_distance(name, c.name)), full)
        })
        .collect();
    let best = scored.iter().map(|(d, _)| *d).min().unwrap_or(usize::MAX);
    if best > (name.len() / 3).max(2) {
        return Vec::new();
    }
    scored.into_iter().filter(|(d, _)| *d == best).map(|(_, c)| c).take(3).collect()
}

/// Run `lunar explain [code]`.
pub fn run(code: Option<&str>, list: bool) -> ExitCode {
    let code = match code {
        Some(c) if !list => normalize(c),
        _ => {
            print_list();
            return ExitCode::SUCCESS;
        }
    };
    if let Some(md) = lookup(&code) {
        print!("{md}");
        return ExitCode::SUCCESS;
    }
    eprintln!("error: no explanation for `{code}`");
    match suggestions(&code).as_slice() {
        [] => {}
        [one] => eprintln!("help: did you mean `{one}`?"),
        many => eprintln!("help: did you mean one of {}?", many.iter().map(|c| format!("`{c}`")).collect::<Vec<_>>().join(", ")),
    }
    eprintln!("note: run `lunar explain --list` to see every code");
    ExitCode::FAILURE
}

fn print_list() {
    let width = ALL_CODES.iter().map(|c| c.to_string().len()).max().unwrap_or(0);
    for c in ALL_CODES {
        let full = c.to_string();
        let t = lookup(&full).map_or("(no explanation)", title);
        println!("{full:width$}  {t}");
    }
}

/// The note printed after the diagnostics of a failed compilation, pointing
/// at `lunar explain` for the error codes that have an explanation.
pub fn hint(diags: &[Diagnostic]) -> Option<String> {
    let mut codes: Vec<String> = Vec::new();
    for d in diags.iter().filter(|d| d.is_error()) {
        let c = d.code.to_string();
        if lookup(&c).is_some() && !codes.contains(&c) {
            codes.push(c);
        }
    }
    let first = codes.first()?;
    if codes.len() == 1 {
        return Some(format!("For more information about this error, try `lunar explain {first}`."));
    }
    const SHOWN: usize = 4;
    let mut list = codes.iter().take(SHOWN).map(String::as_str).collect::<Vec<_>>().join(", ");
    if codes.len() > SHOWN {
        list.push_str(&format!(" and {} more", codes.len() - SHOWN));
    }
    Some(format!("Some errors have detailed explanations: {list}.\nFor more information about an error, try `lunar explain {first}`."))
}
