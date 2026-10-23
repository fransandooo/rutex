//! Working with others through Git (GitHub, GitLab, Overleaf's Git access…)
//! without learning Git: one "Sync" button commits your changes, brings in
//! everyone else's and sends yours. When two people changed the same lines,
//! Git leaves both versions in the file between <<<<<<< and >>>>>>> markers;
//! once they are resolved, the next Sync finishes the merge.
//!
//! Only the project's own repository is used (not the private one of the
//! version history), and nothing runs in the background: the network is only
//! touched when the user presses Sync.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use crate::i18n::{tr, trf};

fn git(root: &Path) -> Command {
    let mut cmd = crate::compiler::command("git");
    cmd.current_dir(root)
        // Never wait for a password or an editor nobody can see.
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .env("LC_ALL", "C");
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes -o ConnectTimeout=15");
    }
    cmd
}

/// Runs git with a time limit (network operations can hang).
fn run(root: &Path, args: &[&str], limit: Duration) -> Result<Output, String> {
    let mut child = git(root).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().map_err(|e| e.to_string()),
            Ok(None) if started.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(tr("Git took too long (is the network down?).").into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(30)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn ok(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = run(root, args, Duration::from_secs(20))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(explain_failure(&String::from_utf8_lossy(&out.stderr)))
    }
}

/// Git's error output, in words a non-Git user can act on.
fn explain_failure(stderr: &str) -> String {
    let s = stderr.trim();
    if s.contains("Please tell me who you are") || s.contains("empty ident") {
        tr("Git does not know your name yet. In a terminal run: git config --global user.name \"Your Name\" and git config --global user.email you@example.com").into()
    } else if s.contains("Permission denied") || s.contains("Authentication failed") || s.contains("could not read Username") || s.contains("terminal prompts disabled") {
        tr("The remote refused the connection. Check that you have access and that your SSH key or credentials are set up for Git.").into()
    } else if s.contains("Could not resolve host") || s.contains("Connection timed out") || s.contains("unable to access") {
        tr("Could not reach the remote. Check your internet connection.").into()
    } else {
        s.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("git failed").trim_start_matches("fatal: ").trim_start_matches("error: ").to_string()
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub branch: String,
    pub has_remote: bool,
    pub upstream: bool,
    /// Commits to send / to receive (as of the last sync).
    pub ahead: u32,
    pub behind: u32,
    /// Files changed and not yet committed.
    pub changed: u32,
    /// Files with unresolved conflicts.
    pub conflicts: Vec<PathBuf>,
    pub merging: bool,
}

/// Status of the repository the project lives in, or None if it is not one.
pub fn status(root: &Path) -> Option<Status> {
    let top = ok(root, &["rev-parse", "--show-toplevel"]).ok()?;
    let top = PathBuf::from(top.trim());
    let out = ok(root, &["status", "--porcelain=v2", "--branch"]).ok()?;
    let mut st = parse_status(&out);
    st.has_remote = ok(root, &["remote"]).is_ok_and(|r| !r.trim().is_empty());
    st.merging = ok(root, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_ok();
    // Conflicts are reported relative to the repository's top level.
    st.conflicts = st.conflicts.into_iter().map(|p| top.join(p)).collect();
    Some(st)
}

pub fn parse_status(porcelain: &str) -> Status {
    let mut st = Status::default();
    for line in porcelain.lines() {
        if let Some(b) = line.strip_prefix("# branch.head ") {
            st.branch = b.to_string();
        } else if let Some(ab) = line.strip_prefix("# branch.ab ") {
            // Only present when the upstream branch exists on the remote (a
            // freshly cloned empty repository names one that does not yet).
            st.upstream = true;
            for part in ab.split_whitespace() {
                if let Some(n) = part.strip_prefix('+') {
                    st.ahead = n.parse().unwrap_or(0);
                } else if let Some(n) = part.strip_prefix('-') {
                    st.behind = n.parse().unwrap_or(0);
                }
            }
        } else if let Some(rest) = line.strip_prefix("u ") {
            // u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>
            if let Some(path) = rest.splitn(10, ' ').nth(9) {
                st.conflicts.push(PathBuf::from(path));
            }
        } else if line.starts_with("1 ") || line.starts_with("2 ") || line.starts_with("? ") {
            st.changed += 1;
        }
    }
    st
}

/// Whether a file still has conflict markers.
fn has_markers(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|t| t.lines().any(|l| l.starts_with("<<<<<<< ") || l.starts_with(">>>>>>> ")))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Everything sent and received: (commits received, commits sent).
    Synced { received: u32, sent: u32 },
    /// Same lines changed on both sides; these files have both versions marked.
    Conflicts(Vec<PathBuf>),
}

/// Commit, pull (merge) and push (blocking).
pub fn sync(root: &Path, message: &str) -> Result<Outcome, String> {
    let st = status(root).ok_or_else(|| tr("This project is not in a Git repository.").to_string())?;
    if !st.has_remote {
        return Err(tr("This repository has no remote to sync with. Add one (for example a GitHub repository) with git remote add origin <url>.").into());
    }
    if st.branch == "(detached)" {
        return Err(tr("The repository is not on a branch (detached HEAD). Check out a branch first.").into());
    }
    // 1. Finish a merge whose conflicts the user has resolved.
    if st.merging {
        let pending: Vec<PathBuf> = st.conflicts.iter().filter(|p| has_markers(p)).cloned().collect();
        if !pending.is_empty() {
            return Ok(Outcome::Conflicts(pending));
        }
        ok(root, &["add", "-A"])?;
        ok(root, &["commit", "--no-edit"])?;
    }
    // 2. Your changes, only inside the project folder (it may be part of a
    //    bigger repository with other work in progress).
    if st.changed > 0 {
        ok(root, &["add", "-A", "--", "."])?;
        let staged = run(root, &["diff", "--cached", "--quiet"], Duration::from_secs(20))?;
        if !staged.status.success() {
            ok(root, &["commit", "-m", message])?;
        }
    }
    let net = Duration::from_secs(90);
    let before = ok(root, &["rev-parse", "HEAD"]).unwrap_or_default();
    // 3. Theirs. A branch without upstream is published first.
    let mut received = 0;
    if st.upstream {
        let out = run(root, &["pull", "--no-rebase", "--no-edit"], net)?;
        if !out.status.success() {
            let after = status(root).unwrap_or_default();
            if !after.conflicts.is_empty() {
                return Ok(Outcome::Conflicts(after.conflicts));
            }
            return Err(explain_failure(&String::from_utf8_lossy(&out.stderr)));
        }
        received = ok(root, &["rev-list", "--count", "--no-merges", &format!("{}..HEAD", before.trim())]).ok().and_then(|n| n.trim().parse().ok()).unwrap_or(0);
    }
    // 4. Yours.
    let sent = ok(root, &["rev-list", "--count", "--no-merges", "@{upstream}..HEAD"]).ok().and_then(|n| n.trim().parse().ok()).unwrap_or(1);
    let push: &[&str] = if st.upstream { &["push"] } else { &["push", "-u", "origin", "HEAD"] };
    let out = run(root, push, net)?;
    if !out.status.success() {
        return Err(explain_failure(&String::from_utf8_lossy(&out.stderr)));
    }
    Ok(Outcome::Synced { received, sent: if st.upstream { sent } else { 1 } })
}

/// Default commit message for Sync.
pub fn message() -> String {
    trf("Update from rutex ({})", &[&glib_now()])
}

fn glib_now() -> String {
    gtk::glib::DateTime::now_local().and_then(|d| d.format("%Y-%m-%d %H:%M")).map(|s| s.to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parses_porcelain() {
        let st = parse_status("# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -1\n1 .M N... 100644 100644 100644 a b main.tex\n? notes.txt\nu UU N... 100644 100644 100644 100644 a b c chapters/intro.tex\n");
        assert_eq!(st.branch, "main");
        assert!(st.upstream);
        assert_eq!((st.ahead, st.behind, st.changed), (2, 1, 2));
        assert_eq!(st.conflicts, [PathBuf::from("chapters/intro.tex")]);
    }

    fn sh(dir: &Path, args: &[&str]) {
        let out = Command::new("git").current_dir(dir).args(args).env("GIT_AUTHOR_NAME", "T").env("GIT_AUTHOR_EMAIL", "t@t").env("GIT_COMMITTER_NAME", "T").env("GIT_COMMITTER_EMAIL", "t@t").output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    /// Two people working on the same project through a shared remote.
    #[test]
    fn syncs_two_people() {
        let base = std::env::temp_dir().join(format!("rutex-sync-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let (remote, ana, ben) = (base.join("remote.git"), base.join("ana"), base.join("ben"));
        sh(&base, &["init", "-q", "--bare", "-b", "main", "remote.git"]);
        sh(&base, &["clone", "-q", remote.to_str().unwrap(), "ana"]);
        for d in [&ana] {
            sh(d, &["config", "user.name", "Ana"]);
            sh(d, &["config", "user.email", "ana@x"]);
        }
        fs::write(ana.join("main.tex"), "one\ntwo\nthree\n").unwrap();
        // No upstream yet: the first sync publishes the branch.
        assert_eq!(sync(&ana, "first").unwrap(), Outcome::Synced { received: 0, sent: 1 });
        sh(&base, &["clone", "-q", remote.to_str().unwrap(), "ben"]);
        sh(&ben, &["config", "user.name", "Ben"]);
        sh(&ben, &["config", "user.email", "ben@x"]);

        // Ben changes line 1, Ana line 3: both merge cleanly.
        fs::write(ben.join("main.tex"), "ONE\ntwo\nthree\n").unwrap();
        assert!(matches!(sync(&ben, "ben").unwrap(), Outcome::Synced { .. }));
        fs::write(ana.join("main.tex"), "one\ntwo\nTHREE\n").unwrap();
        assert_eq!(sync(&ana, "ana").unwrap(), Outcome::Synced { received: 1, sent: 1 });
        assert_eq!(fs::read_to_string(ana.join("main.tex")).unwrap(), "ONE\ntwo\nTHREE\n");

        // Both change line 2: a conflict, then resolved and synced.
        assert!(matches!(sync(&ben, "ben").unwrap(), Outcome::Synced { .. }));
        fs::write(ben.join("main.tex"), "ONE\nben\nTHREE\n").unwrap();
        assert!(matches!(sync(&ben, "ben").unwrap(), Outcome::Synced { .. }));
        fs::write(ana.join("main.tex"), "ONE\nana\nTHREE\n").unwrap();
        match sync(&ana, "ana").unwrap() {
            Outcome::Conflicts(files) => assert_eq!(files, [fs::canonicalize(&ana).unwrap().join("main.tex")]),
            other => panic!("{other:?}"),
        }
        assert!(matches!(sync(&ana, "again").unwrap(), Outcome::Conflicts(_)), "markers still there");
        fs::write(ana.join("main.tex"), "ONE\nana and ben\nTHREE\n").unwrap();
        assert!(matches!(sync(&ana, "resolved").unwrap(), Outcome::Synced { .. }));
        assert!(matches!(sync(&ben, "ben").unwrap(), Outcome::Synced { received: 1.., .. }));
        assert_eq!(fs::read_to_string(ben.join("main.tex")).unwrap(), "ONE\nana and ben\nTHREE\n");

        // A project in a subfolder only sends its own files.
        fs::create_dir_all(ana.join("thesis")).unwrap();
        fs::write(ana.join("thesis/main.tex"), "x\n").unwrap();
        fs::write(ana.join("unrelated.txt"), "draft\n").unwrap();
        assert!(matches!(sync(&ana.join("thesis"), "thesis").unwrap(), Outcome::Synced { .. }));
        let pending = parse_status(&ok(&ana, &["status", "--porcelain=v2", "--branch"]).unwrap());
        assert_eq!(pending.changed, 1, "unrelated.txt stays uncommitted");
        fs::remove_dir_all(base).unwrap();
    }
}
