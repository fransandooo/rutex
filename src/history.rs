//! Local version history. Each project gets a private git repository in
//! ~/.local/share/rutex/history/<project>, used with `--git-dir`/`--work-tree`
//! so nothing is written inside the project folder and the user's own git
//! repository (if any) is never touched. Nothing leaves the machine.
//!
//! All functions here block (they run git) and are meant for a worker thread.

use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::compiler;

/// Subject prefix of versions saved by the user (the rest is their name).
const NAMED: &str = "named: ";
/// Files and folders never stored in the history.
const EXCLUDE: &str = ".git/\n.*\n!.latexmkrc\n*.rutex~\nnode_modules/\n__pycache__/\n_minted*/\n*.synctex.gz\n";

#[derive(Clone, Debug, PartialEq)]
pub struct Version {
    pub id: String,
    /// Seconds since the epoch.
    pub time: i64,
    /// Name given by the user; None for automatic snapshots.
    pub name: Option<String>,
    pub files_changed: usize,
}

pub struct History {
    git_dir: PathBuf,
    work_tree: PathBuf,
}

pub fn git_available() -> bool {
    compiler::command("git").arg("--version").output().is_ok_and(|o| o.status.success())
}

impl History {
    pub fn for_project(root: &Path, build_dir: &Path) -> History {
        // Same key as the build cache, so the folder name is stable and unique.
        let key = build_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        History { git_dir: gtk::glib::user_data_dir().join("rutex").join("history").join(key), work_tree: root.to_path_buf() }
    }

    fn git(&self, args: &[&str]) -> std::io::Result<Output> {
        compiler::command("git")
            .env("GIT_DIR", &self.git_dir)
            .env("GIT_WORK_TREE", &self.work_tree)
            .env("GIT_AUTHOR_NAME", "rutex")
            .env("GIT_AUTHOR_EMAIL", "rutex@localhost")
            .env("GIT_COMMITTER_NAME", "rutex")
            .env("GIT_COMMITTER_EMAIL", "rutex@localhost")
            .env("GIT_TERMINAL_PROMPT", "0")
            // Ignore the user's hooks, signing and line-ending settings.
            .args(["-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false", "-c", "core.autocrlf=false", "-c", "core.quotepath=false"])
            .args(args)
            .current_dir(&self.work_tree)
            .output()
    }

    fn ok(&self, args: &[&str]) -> std::io::Result<String> {
        let out = self.git(args)?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(std::io::Error::other(String::from_utf8_lossy(&out.stderr).trim().to_string()))
        }
    }

    fn ensure(&self) -> std::io::Result<()> {
        if self.git_dir.join("HEAD").exists() {
            return Ok(());
        }
        std::fs::create_dir_all(&self.git_dir)?;
        self.ok(&["init", "--quiet"])?;
        std::fs::create_dir_all(self.git_dir.join("info"))?;
        std::fs::write(self.git_dir.join("info").join("exclude"), EXCLUDE)?;
        Ok(())
    }

    /// Saves the current state if anything changed. Returns whether a version was created.
    pub fn snapshot(&self, name: Option<&str>) -> std::io::Result<bool> {
        self.ensure()?;
        self.ok(&["add", "--all", "."])?;
        let has_head = self.git(&["rev-parse", "--verify", "--quiet", "HEAD"])?.status.success();
        let changed = !has_head || !self.git(&["diff", "--cached", "--quiet"])?.status.success();
        // A named version is always recorded, even without changes.
        if !changed && name.is_none() {
            return Ok(false);
        }
        let subject = match name {
            Some(n) => format!("{NAMED}{}", n.trim()),
            None => "auto".to_string(),
        };
        self.ok(&["commit", "--quiet", "--allow-empty", "-m", &subject])?;
        Ok(true)
    }

    /// Seconds since the last version (None if there is none).
    pub fn age_of_last(&self) -> Option<i64> {
        let t: i64 = self.ok(&["log", "-1", "--format=%ct"]).ok()?.trim().parse().ok()?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
        Some(now - t)
    }

    pub fn versions(&self, limit: usize) -> Vec<Version> {
        if !self.git_dir.join("HEAD").exists() {
            return Vec::new();
        }
        let limit = format!("-{limit}");
        let Ok(log) = self.ok(&["log", &limit, "--format=\u{1e}%H\u{1f}%ct\u{1f}%s", "--shortstat"]) else { return Vec::new() };
        parse_log(&log)
    }

    /// Unified diff of the project between a version and the current files.
    pub fn diff_with_current(&self, id: &str) -> String {
        // Include files created since then.
        let _ = self.git(&["add", "--all", "--intent-to-add", "."]);
        self.ok(&["diff", "--no-color", "--find-renames", id]).unwrap_or_default()
    }

    /// Makes the project folder match `id`, after saving the current state so
    /// the restore itself can be undone.
    pub fn restore(&self, id: &str, label: &str) -> std::io::Result<()> {
        self.snapshot(Some(label))?;
        self.ok(&["restore", "--source", id, "--staged", "--worktree", "--", "."])?;
        Ok(())
    }
}

fn parse_log(log: &str) -> Vec<Version> {
    log.split('\u{1e}')
        .filter(|r| !r.trim().is_empty())
        .filter_map(|record| {
            let mut lines = record.lines();
            let mut fields = lines.next()?.split('\u{1f}');
            let id = fields.next()?.to_string();
            let time = fields.next()?.parse().ok()?;
            let subject = fields.next().unwrap_or("");
            let name = subject.strip_prefix(NAMED).map(str::to_string);
            // " 3 files changed, 10 insertions(+)"
            let files_changed = lines
                .find(|l| l.contains("changed"))
                .and_then(|l| l.split_whitespace().next()?.parse().ok())
                .unwrap_or(0);
            Some(Version { id, time, name, files_changed })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_git_log() {
        let log = "\u{1e}abc\u{1f}1700000000\u{1f}named: Before review\n\n 2 files changed, 5 insertions(+)\n\u{1e}def\u{1f}1699990000\u{1f}auto\n\n 1 file changed, 1 deletion(-)\n";
        let v = parse_log(log);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0], Version { id: "abc".into(), time: 1700000000, name: Some("Before review".into()), files_changed: 2 });
        assert_eq!(v[1].name, None);
        assert_eq!(v[1].files_changed, 1);
    }

    /// Full round trip on a temporary project: snapshot, edit, diff, restore.
    #[test]
    fn snapshot_diff_and_restore() {
        if !git_available() {
            return;
        }
        let base = std::env::temp_dir().join(format!("rutex-history-{}", std::process::id()));
        let root = base.join("project");
        std::fs::create_dir_all(root.join("chapters")).unwrap();
        std::fs::write(root.join("main.tex"), "uno\n").unwrap();
        std::fs::write(root.join("chapters/a.tex"), "a\n").unwrap();
        // The user's own git folder must not end up in the history.
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/config"), "secret").unwrap();
        let h = History { git_dir: base.join("history"), work_tree: root.clone() };

        assert!(h.snapshot(Some("First")).unwrap());
        assert!(!h.snapshot(None).unwrap(), "no changes, no automatic version");
        let first = h.versions(10)[0].clone();
        assert_eq!(first.name.as_deref(), Some("First"));

        std::fs::write(root.join("main.tex"), "dos\n").unwrap();
        std::fs::write(root.join("new.tex"), "nuevo\n").unwrap();
        let diff = h.diff_with_current(&first.id);
        assert!(diff.contains("-uno") && diff.contains("+dos") && diff.contains("new.tex"), "{diff}");
        assert!(!diff.contains(".git/config"));

        h.restore(&first.id, "Before restoring").unwrap();
        assert_eq!(std::fs::read_to_string(root.join("main.tex")).unwrap(), "uno\n");
        assert!(!root.join("new.tex").exists(), "files created later are removed");
        assert!(root.join(".git/config").exists(), "the user's .git is untouched");
        // The state before restoring was saved, so the restore can be undone.
        let versions = h.versions(10);
        assert_eq!(versions[0].name.as_deref(), Some("Before restoring"));
        std::fs::remove_dir_all(base).unwrap();
    }
}
