//! TECH-bug-prevention (а), TEAM.md §8d-4: in the core's and the chain's own code a read that
//! failed is never quietly taken as zero or empty. Every `.unwrap_or(0)`, `.unwrap_or(0.0)` and
//! `.unwrap_or_default()` outside tests carries `// default-ok: <why>` on its line or the line
//! above: a reason a default is right there (a count shown, a list that is honestly empty, a
//! fallback a later read corrects). Money and state of the core take three states instead:
//! «known», «none», «not known yet».

use std::path::{Path, PathBuf};

/// The patterns that turn a failure into a value.
const DEFAULTS: [&str; 3] = [".unwrap_or(0)", ".unwrap_or(0.0)", ".unwrap_or_default()"];

/// The mark that says a default is right there, and why.
const MARK: &str = "default-ok:";

/// The Rust files under `dir`, tests left out (a `tests` folder, `tests.rs`, `*_tests.rs`).
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default(); // default-ok: a name the test cannot read is skipped
        if path.is_dir() {
            if name != "tests" {
                sources(&path, out);
            }
        } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
            out.push(path);
        }
    }
}

/// The lines of `text` with its test modules (`#[cfg(test)]` followed by a `mod`, up to the
/// brace that closes it) blanked: the file's own code, each line where it was. A test module in
/// the middle of a file hides only itself (review r1: one at line 44 of `snapshot.rs` hid the
/// two thousand lines after it).
fn own_code(text: &str) -> Vec<&str> {
    let mut lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i + 1 < lines.len() {
        if lines[i].trim() != "#[cfg(test)]" || !lines[i + 1].trim_start().starts_with("mod ") {
            i += 1;
            continue;
        }
        // From the attribute to the module's closing brace (`mod x;` is one line).
        let mut depth = 0i64;
        let mut j = i + 1;
        while j < lines.len() {
            let code = lines[j].find("//").map_or(lines[j], |k| &lines[j][..k]);
            depth += code.matches('{').count() as i64 - code.matches('}').count() as i64;
            if depth <= 0 && (code.contains('}') || code.trim_end().ends_with(';')) {
                break;
            }
            j += 1;
        }
        let end = j.min(lines.len() - 1);
        for line in &mut lines[i..=end] {
            *line = "";
        }
        i = end + 1;
    }
    lines
}

/// `file:line: text` of every default in `text` without its mark.
fn unmarked(path: &Path, text: &str) -> Vec<String> {
    let lines = own_code(text);
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with("//") || !DEFAULTS.iter().any(|d| line.contains(d)) {
            continue;
        }
        // A mark above counts only as a comment line of its own: another line's own mark does
        // not cover the next one.
        let above = i
            .checked_sub(1)
            .map(|j| lines[j].trim())
            .unwrap_or_default(); // default-ok: the first line has none above
        if !line.contains(MARK) && !(above.starts_with("//") && above.contains(MARK)) {
            found.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
        }
    }
    found
}

/// §8d-4: no default without its reason in the core's (`src/real`) and the chain's code. A new
/// one fails here until it is a three-state answer or carries `// default-ok: <why>`.
#[test]
fn no_failure_is_quietly_a_default() {
    let core = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    sources(&core.join("src").join("real"), &mut files);
    sources(
        &core.join("..").join("evolite-chain").join("src"),
        &mut files,
    );
    assert!(files.len() > 50, "the sources were found: {}", files.len());
    let mut found = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        found.extend(unmarked(file, &text));
    }
    assert!(
        found.is_empty(),
        "{} defaults without `// {MARK} <why>` (TEAM.md §8d-4):\n{}",
        found.len(),
        found.join("\n")
    );
}

/// The guard's own rules: a mark on the line or the line above passes, a comment is not code,
/// and the test module is not looked at.
#[test]
fn the_guard_reads_marks_and_skips_tests() {
    let text = "let a = x.unwrap_or(0);\n\
                let b = y.unwrap_or(0); // default-ok: a count shown\n\
                let e = v.unwrap_or(0);\n\
                // default-ok: an empty list is the honest answer\n\
                let c = z.unwrap_or_default();\n\
                // a comment: x.unwrap_or(0)\n\
                #[cfg(test)]\n\
                mod tests {\n\
                    fn t() { let d = w.unwrap_or(0); }\n\
                }\n\
                let f = u.unwrap_or_default();\n\
                #[cfg(test)]\n\
                mod more { let g = w.unwrap_or(0); }\n";
    let found = unmarked(Path::new("f.rs"), text);
    assert_eq!(
        found,
        vec![
            "f.rs:1: let a = x.unwrap_or(0);".to_string(),
            "f.rs:3: let e = v.unwrap_or(0);".to_string(),
            "f.rs:11: let f = u.unwrap_or_default();".to_string(),
        ],
        "the mark of line 2 does not cover line 3; code after a test module is looked at"
    );
}
