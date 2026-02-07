# dirty

A fast CLI that scans a directory for git repos and shows which ones have uncommitted changes and which are local-only (no remote).

```
$ dirty ~/code
 * apps/dashboard
   apps/storefront
 * libs/ui-kit [local]
   libs/common
 * services/auth
   services/payments
   tools/cli [local]
 * tools/scripts [local]

8 repos, 4 dirty, 3 local-only
```

- `*` — repo has uncommitted changes (red)
- `[local]` — repo has no remotes configured (yellow)

## Install

```sh
cargo install --git https://github.com/iamkaf/dirty
```

## Usage

```
dirty <path>              # scan (default depth: 3)
dirty -L 1 <path>         # scan immediate subdirectories only
dirty -d <path>           # only dirty repos
dirty -l <path>           # only local-only repos
dirty -dlr <path>         # dirty + local, raw paths for piping
```

| Flag | Short | Default | Description |
|------|-------|---------|-------------|
| `--depth` | `-L` | `3` | Max directory depth to search for repos |
| `--dirty` | `-d` | off | Only show repos with uncommitted changes |
| `--local` | `-l` | off | Only show repos with no remotes |
| `--raw` | `-r` | off | One path per line, no decorations |

## How it works

1. Walks directories up to the specified depth looking for `.git` folders
2. Inspects each repo in parallel using libgit2 (via [git2](https://crates.io/crates/git2)) and [rayon](https://crates.io/crates/rayon)
3. Checks for uncommitted/untracked changes and whether any remotes exist
