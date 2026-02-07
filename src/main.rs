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
    #[arg(short = 'L', default_value = "1")]
    depth: usize,

    /// Only show dirty repos
    #[arg(long)]
    dirty: bool,

    /// Only show local-only repos (no remotes)
    #[arg(long)]
    local: bool,
}

struct RepoInfo {
    path: PathBuf,
    dirty: bool,
    local_only: bool,
}

fn find_repos(base: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut repos = Vec::new();
    collect_repos(base, max_depth, 0, &mut repos);
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
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
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
    let statuses = repo.statuses(Some(&mut opts)).ok()?;
    let dirty = !statuses.is_empty();

    let local_only = repo.remotes().ok().map_or(true, |r| r.is_empty());

    Some(RepoInfo {
        path: path.to_path_buf(),
        dirty,
        local_only,
    })
}

fn main() {
    let args = Args::parse();
    let base = args.path.canonicalize().unwrap_or_else(|_| {
        eprintln!("dirty: cannot access '{}'", args.path.display());
        std::process::exit(1);
    });

    let mut repos = find_repos(&base, args.depth);
    repos.sort();

    let infos: Vec<_> = repos.par_iter().filter_map(|p| inspect_repo(p)).collect();

    if infos.is_empty() {
        eprintln!("No git repos found in {}", base.display());
        return;
    }

    let infos: Vec<_> = infos
        .into_iter()
        .filter(|i| (!args.dirty || i.dirty) && (!args.local || i.local_only))
        .collect();

    if infos.is_empty() {
        eprintln!("No matching repos found");
        return;
    }

    for info in &infos {
        let rel = info
            .path
            .strip_prefix(&base)
            .unwrap_or(&info.path)
            .display();

        let dirty_marker = if info.dirty { "\x1b[31m*\x1b[0m" } else { " " };
        let local_marker = if info.local_only {
            " \x1b[33m[local]\x1b[0m"
        } else {
            ""
        };

        println!(" {dirty_marker} {rel}{local_marker}");
    }

    let dirty_count = infos.iter().filter(|i| i.dirty).count();
    let local_count = infos.iter().filter(|i| i.local_only).count();
    println!(
        "\n{} repos, {dirty_count} dirty, {local_count} local-only",
        infos.len()
    );
}
