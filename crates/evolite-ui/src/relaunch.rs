//! The one place evolite starts its own executable again: after a resync or for a new Core node
//! (`app::start_again`), and the network switch (`app::network`). In tests nothing starts: a test
//! binary started again runs the suite once more, outside the harness and beside the person's
//! windows — during batch 69's check (2026-11-22) a demo evolite window came up over QA's. The
//! start is kept for [`started`] instead, and `no_test_starts_its_own_executable` holds every
//! other file to this one.

use std::ffi::OsString;

/// Whether the process started has ended, asked again and again.
pub type Ended = Box<dyn FnMut() -> bool>;

/// Starts this executable with `args`.
pub fn start(args: Vec<OsString>) -> std::io::Result<Ended> {
    #[cfg(test)]
    {
        STARTED.with(|s| s.borrow_mut().push(args));
        Ok(Box::new(|| false))
    }
    #[cfg(not(test))]
    {
        let mut child = std::process::Command::new(std::env::current_exe()?)
            .args(args)
            .spawn()?;
        Ok(Box::new(move || matches!(child.try_wait(), Ok(Some(_)))))
    }
}

#[cfg(test)]
thread_local! {
    static STARTED: std::cell::RefCell<Vec<Vec<OsString>>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The starts this thread asked for, in order (tests only).
#[cfg(test)]
pub fn started() -> Vec<Vec<OsString>> {
    STARTED.with(|s| s.borrow().clone())
}

#[cfg(test)]
mod tests {
    /// A test never starts its own executable: [`super::start`] keeps the arguments instead.
    #[test]
    fn a_start_in_a_test_is_only_kept() {
        let before = super::started().len();
        let mut ended = super::start(vec!["--screen".into(), "overview".into()]).unwrap();
        assert!(!ended(), "nothing started, nothing ends");
        let all = super::started();
        assert_eq!(all.len(), before + 1);
        assert_eq!(
            all.last().unwrap(),
            &vec!["--screen".into(), "overview".into()] as &Vec<std::ffi::OsString>
        );
    }

    /// The guard (2026-11-22, a demo window over QA's during a check; the architect's r1 Should):
    /// no source of the app starts a program — its own executable (`current_exe()`), another
    /// one (`Command::new`) or the Windows shell's «execute» — but in the places named here, each
    /// with its reason. A new one fails here: start through [`super::start`], or keep it out of
    /// tests (`cfg(not(test))`) and name it.
    #[test]
    fn no_test_starts_a_program() {
        // (file, what it may contain, why a test does not start anything there)
        const NAMED: &[(&str, &str, &str)] = &[
            (
                "relaunch.rs",
                "current_exe",
                "the one start, nothing under cfg(test)",
            ),
            (
                "relaunch.rs",
                "Command::new",
                "the one start, nothing under cfg(test)",
            ),
            (
                "crash_reports.rs",
                "current_exe",
                "Windows Error Reporting's exclusion by the file's path: no start",
            ),
            (
                "links.rs",
                "Command::new",
                "a link or a folder: nothing under cfg(test)",
            ),
            (
                "links.rs",
                "shell execute",
                "a link on Windows: nothing under cfg(test)",
            ),
            (
                "screens/settings/report.rs",
                "Command::new",
                "the reports' folder: nothing under cfg(test)",
            ),
            (
                "diagnostics.rs",
                "Command::new",
                "macOS's `sw_vers` for the version line: no window",
            ),
            (
                "i18n/mod.rs",
                "Command::new",
                "macOS's `defaults` for the system language: no window",
            ),
            (
                "setup/main.rs",
                "current_exe",
                "the installer's own binary: where its payload is",
            ),
            (
                "setup/work.rs",
                "current_exe",
                "the installer: the folder it runs from",
            ),
            (
                "setup/work.rs",
                "Command::new",
                "the installer's tests: ping, sleep and lsof, no window",
            ),
            (
                "setup/system.rs",
                "Command::new",
                "the installer: cmd, ps, the system's dialogs, run by hand",
            ),
            (
                "setup/ui.rs",
                "Command::new",
                "the installer's «Open evolite» after an install, its own binary",
            ),
        ];
        let needles = [
            ("current_exe", ["current_exe", "()"].concat()),
            ("Command::new", ["Command::", "new("].concat()),
            ("shell execute", ["Shell", "Execute"].concat()),
        ];
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        let mut dirs = vec![src.clone()];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).expect("a source folder").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let rel = path
                    .strip_prefix(&src)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let text = std::fs::read_to_string(&path).expect("a source file");
                for (name, needle) in &needles {
                    let named = NAMED.iter().any(|(f, n, _)| *f == rel && n == name);
                    if text.contains(needle.as_str()) && !named {
                        found.push(format!("{rel}: {name}"));
                    }
                }
            }
        }
        // And back: a name left here after its start went is a hole kept open.
        let gone: Vec<String> = NAMED
            .iter()
            .filter(|(f, n, _)| {
                let text = std::fs::read_to_string(src.join(f)).unwrap_or_default();
                let needle = &needles.iter().find(|(name, _)| name == n).unwrap().1;
                !text.contains(needle.as_str())
            })
            .map(|(f, n, _)| format!("{f}: {n}"))
            .collect();
        assert!(
            found.is_empty(),
            "a start a test could run: through relaunch::start, or under cfg(not(test)) and named: {found:?}"
        );
        assert!(gone.is_empty(), "named, no longer there: {gone:?}");
    }
}
