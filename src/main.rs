use clap::Parser;
use git2::{BranchType, Repository, StatusOptions};
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(about = "List git repos, their dirty status, and whether they're local-only")]
struct Args {
    /// Directory to scan
    path: PathBuf,

    /// Max depth to search for repos
    #[arg(short = 'L', default_value = "3")]
    depth: usize,

    /// Only show dirty repos
    #[arg(short, long)]
    dirty: bool,

    /// Only show local-only repos (no remotes)
    #[arg(short, long)]
    local: bool,

    /// Include unpushed commit info (ahead of upstream) in the output
    ///
    /// Note: this requires resolving the upstream tracking branch, which is slower,
    /// so it is only computed when this flag is set.
    #[arg(short = 'u', long)]
    include_unpushed: bool,

    /// Show the current branch in the output
    #[arg(short = 'b', long)]
    branch: bool,

    /// Raw output for piping (one path per line)
    #[arg(short, long)]
    raw: bool,
}

struct RepoInfo {
    path: PathBuf,
    dirty: bool,
    local_only: bool,
    branch: Option<String>,
    ahead: Option<usize>,
}

fn find_repos(base: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut repos = Vec::new();
    collect_repos(base, max_depth, 0, &mut repos);
    repos.sort();
    repos
}

fn collect_repos(dir: &Path, max_depth: usize, depth: usize, repos: &mut Vec<PathBuf>) {
    if depth > max_depth {
        return;
    }
    if dir.join(".git").exists() {
        repos.push(dir.to_path_buf());
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && !path.is_symlink() {
            collect_repos(&path, max_depth, depth + 1, repos);
        }
    }
}

fn ahead_of_upstream(repo: &Repository) -> Option<usize> {
    // Detached HEAD or unborn branch will fail here.
    let head = repo.head().ok()?;
    let head_oid = head.target()?;

    // Resolve upstream tracking branch via Branch API.
    let name = head.shorthand()?;
    let branch = repo.find_branch(name, BranchType::Local).ok()?;
    let upstream = branch.upstream().ok()?;

    let upstream_ref = upstream.get();
    let upstream_oid = upstream_ref.target()?;

    // ahead/behind count vs upstream
    let (ahead, _behind) = repo.graph_ahead_behind(head_oid, upstream_oid).ok()?;
    Some(ahead)
}

fn current_branch(repo: &Repository) -> Option<String> {
    let head = repo.head().ok()?;
    if head.is_branch() {
        return head.shorthand().map(str::to_owned);
    }

    let oid = head.target()?;
    Some(format!("detached:{}", &oid.to_string()[..7]))
}

fn inspect_repo(path: &Path, compute_unpushed: bool, compute_branch: bool) -> Option<RepoInfo> {
    let repo = Repository::open(path).ok()?;

    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(false)
        .exclude_submodules(true);
    let dirty = !repo.statuses(Some(&mut opts)).ok()?.is_empty();
    let local_only = repo.remotes().ok().is_none_or(|r| r.is_empty());

    let ahead = if compute_unpushed {
        ahead_of_upstream(&repo)
    } else {
        None
    };
    let branch = if compute_branch {
        current_branch(&repo)
    } else {
        None
    };

    Some(RepoInfo {
        path: path.to_path_buf(),
        dirty,
        local_only,
        branch,
        ahead,
    })
}

fn run(args: Args) -> Result<(), String> {
    let base = args
        .path
        .canonicalize()
        .map_err(|_| format!("dirty: cannot access '{}'", args.path.display()))?;

    let repos = find_repos(&base, args.depth);
    let infos: Vec<_> = repos
        .par_iter()
        .filter_map(|p| inspect_repo(p, args.include_unpushed, args.branch))
        .filter(|i| (!args.dirty || i.dirty) && (!args.local || i.local_only))
        .collect();

    if infos.is_empty() {
        return Err(if repos.is_empty() {
            format!("No git repos found in {}", base.display())
        } else {
            "No matching repos found".into()
        });
    }

    for info in &infos {
        let rel = info
            .path
            .strip_prefix(&base)
            .unwrap_or(&info.path)
            .display();
        if args.raw {
            println!("{rel}");
        } else {
            let dirty = if info.dirty { "\x1b[31m*\x1b[0m" } else { " " };
            let branch = info
                .branch
                .as_ref()
                .map(|name| format!(" \x1b[36m[{name}]\x1b[0m"))
                .unwrap_or_default();
            let local = if info.local_only {
                " \x1b[33m[local]\x1b[0m"
            } else {
                ""
            };
            let unpushed = if args.include_unpushed {
                match info.ahead {
                    Some(n) if n > 0 => {
                        // blue
                        format!(" \x1b[34m[↑{n}]\x1b[0m")
                    }
                    _ => String::new(),
                }
            } else {
                String::new()
            };
            println!(" {dirty} {rel}{branch}{local}{unpushed}");
        }
    }

    if !args.raw {
        let dirty_count = infos.iter().filter(|i| i.dirty).count();
        let local_count = infos.iter().filter(|i| i.local_only).count();
        println!(
            "\n{} repos, {dirty_count} dirty, {local_count} local-only",
            infos.len()
        );
    }

    Ok(())
}

fn main() {
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    if let Err(e) = run(Args::parse()) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    fn setup_repo(tmp: &Path, name: &str, dirty: bool, add_remote: bool) -> PathBuf {
        let dir = tmp.join(name);
        fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        git(&dir, &["config", "user.email", "dirty@example.invalid"]);
        git(&dir, &["config", "user.name", "Dirty Tests"]);
        // need an initial commit so status works cleanly
        git(&dir, &["commit", "--allow-empty", "-m", "init", "-q"]);
        if add_remote {
            git(
                &dir,
                &["remote", "add", "origin", "https://example.com/repo.git"],
            );
        }
        if dirty {
            fs::write(dir.join("untracked.txt"), "hello").unwrap();
        }
        dir
    }

    #[test]
    fn find_repos_respects_depth() {
        let tmp = tempfile::tempdir().unwrap();
        setup_repo(tmp.path(), "a", false, true);
        setup_repo(tmp.path(), "deep/nested/b", false, true);

        assert_eq!(find_repos(tmp.path(), 1).len(), 1);
        assert_eq!(find_repos(tmp.path(), 3).len(), 2);
    }

    #[test]
    fn inspect_detects_dirty() {
        let tmp = tempfile::tempdir().unwrap();
        let clean = setup_repo(tmp.path(), "clean", false, true);
        let dirty = setup_repo(tmp.path(), "dirty", true, true);

        assert!(!inspect_repo(&clean, false, false).unwrap().dirty);
        assert!(inspect_repo(&dirty, false, false).unwrap().dirty);
    }

    #[test]
    fn inspect_detects_local_only() {
        let tmp = tempfile::tempdir().unwrap();
        let with_remote = setup_repo(tmp.path(), "remote", false, true);
        let no_remote = setup_repo(tmp.path(), "local", false, false);

        assert!(!inspect_repo(&with_remote, false, false).unwrap().local_only);
        assert!(inspect_repo(&no_remote, false, false).unwrap().local_only);
    }

    #[test]
    fn find_repos_skips_nested_inside_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let parent = setup_repo(tmp.path(), "parent", false, true);
        fs::create_dir_all(parent.join("child/.git")).unwrap();

        assert_eq!(find_repos(tmp.path(), 5).len(), 1);
    }

    #[test]
    fn inspect_includes_branch_when_requested() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = setup_repo(tmp.path(), "repo", false, true);
        let branch = Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(&repo)
            .output()
            .unwrap()
            .stdout;
        let branch = String::from_utf8(branch).unwrap().trim().to_owned();

        assert_eq!(
            inspect_repo(&repo, false, true).unwrap().branch,
            Some(branch)
        );
        assert_eq!(inspect_repo(&repo, false, false).unwrap().branch, None);
    }

    #[test]
    fn inspect_shows_detached_head_when_requested() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = setup_repo(tmp.path(), "repo", false, true);
        let oid = Command::new("git")
            .args(["rev-parse", "--short=7", "HEAD"])
            .current_dir(&repo)
            .output()
            .unwrap()
            .stdout;
        let short_oid = String::from_utf8(oid).unwrap().trim().to_owned();
        git(&repo, &["checkout", "--detach", "-q"]);

        assert_eq!(
            inspect_repo(&repo, false, true).unwrap().branch,
            Some(format!("detached:{short_oid}"))
        );
    }
}
