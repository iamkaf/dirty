use clap::Parser;
use git2::{Repository, StatusOptions};
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

    /// Raw output for piping (one path per line)
    #[arg(short, long)]
    raw: bool,
}

struct RepoInfo {
    path: PathBuf,
    dirty: bool,
    local_only: bool,
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

fn inspect_repo(path: &Path) -> Option<RepoInfo> {
    let repo = Repository::open(path).ok()?;

    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(false)
        .exclude_submodules(true);
    let dirty = !repo.statuses(Some(&mut opts)).ok()?.is_empty();
    let local_only = repo.remotes().ok().is_none_or(|r| r.is_empty());

    Some(RepoInfo {
        path: path.to_path_buf(),
        dirty,
        local_only,
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
        .filter_map(|p| inspect_repo(p))
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
        let rel = info.path.strip_prefix(&base).unwrap_or(&info.path).display();
        if args.raw {
            println!("{rel}");
        } else {
            let dirty = if info.dirty { "\x1b[31m*\x1b[0m" } else { " " };
            let local = if info.local_only { " \x1b[33m[local]\x1b[0m" } else { "" };
            println!(" {dirty} {rel}{local}");
        }
    }

    if !args.raw {
        let dirty_count = infos.iter().filter(|i| i.dirty).count();
        let local_count = infos.iter().filter(|i| i.local_only).count();
        println!("\n{} repos, {dirty_count} dirty, {local_count} local-only", infos.len());
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

    fn setup_repo(tmp: &Path, name: &str, dirty: bool, add_remote: bool) -> PathBuf {
        let dir = tmp.join(name);
        fs::create_dir_all(&dir).unwrap();
        Command::new("git").args(["init", "-q"]).current_dir(&dir).status().unwrap();
        // need an initial commit so status works cleanly
        Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init", "-q"])
            .current_dir(&dir)
            .status()
            .unwrap();
        if add_remote {
            Command::new("git")
                .args(["remote", "add", "origin", "https://example.com/repo.git"])
                .current_dir(&dir)
                .status()
                .unwrap();
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

        assert!(!inspect_repo(&clean).unwrap().dirty);
        assert!(inspect_repo(&dirty).unwrap().dirty);
    }

    #[test]
    fn inspect_detects_local_only() {
        let tmp = tempfile::tempdir().unwrap();
        let with_remote = setup_repo(tmp.path(), "remote", false, true);
        let no_remote = setup_repo(tmp.path(), "local", false, false);

        assert!(!inspect_repo(&with_remote).unwrap().local_only);
        assert!(inspect_repo(&no_remote).unwrap().local_only);
    }

    #[test]
    fn find_repos_skips_nested_inside_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let parent = setup_repo(tmp.path(), "parent", false, true);
        fs::create_dir_all(parent.join("child/.git")).unwrap();

        assert_eq!(find_repos(tmp.path(), 5).len(), 1);
    }
}
