# MiniGit Collab

MiniGit Collab is an educational Git-like version control system written in
Rust. It is built to teach content-addressed storage, repository internals,
filesystem programming, error handling, and later collaboration and fuzzing.

## Project Status

Phases 1 and 2 are implemented. MiniGit supports local repositories,
content-addressed objects, staging, commits, status, checkout, branches,
switching, fast-forward merges, three-way merges, and conflict markers. It is a
learning project and is not compatible with real Git repositories.

The server and fuzz crates are placeholders for later phases.

## Architecture

```text
minigit/
  crates/
    minigit-core/    # Object model and repository operations
    minigit-cli/     # Command-line interface
    minigit-server/  # Later phase; not implemented
    minigit-fuzz/    # Later phase; not implemented
  README.md
  SECURITY.md
```

## Build And Test

```bash
cargo build --workspace
cargo test --workspace
```

The compiled CLI is `target/debug/minigit`.

## Commands

Run these commands inside the folder that you want MiniGit to manage:

```bash
minigit init
minigit add <file>
minigit commit -m "message"
minigit log
minigit status
minigit checkout <commit>
minigit branch <name>
minigit switch <branch>
minigit merge <branch>
```

## How It Works

- `add` stores the file contents as a Blob and records its hash in the index.
- `commit` stores the index as a Tree, then stores a Commit pointing to that
  Tree and its parent Commits.
- `HEAD` points to the selected branch ref, which stores its latest Commit
  hash. Checking out a commit directly creates a detached HEAD.
- `branch` creates another ref at the current Commit, and `switch` restores the
  branch snapshot and updates `HEAD`.
- `merge` finds a common ancestor and performs an already-up-to-date,
  fast-forward, or three-way merge.
- Conflicting files receive Git-style markers. After resolving them, run `add`
  for each file and `commit` to create the two-parent merge Commit.
- `log` follows the first parent from newest to oldest.
- SHA-256 object hashes are encoded as 64 hexadecimal characters and used as
  filenames inside `.minigit/objects/`.

```text
HEAD -> branch ref -> Commit -> Tree -> Blobs
```

## Repository Layout

```text
.minigit/
  HEAD
  index.json
  objects/
  refs/
    heads/
      main
      feature
```

During conflict resolution, `.minigit/MERGE_HEAD` and
`.minigit/MERGE_CONFLICTS` temporarily record merge state.

## Phase 2

Phase 2 is implemented:

```bash
minigit branch <name>
minigit switch <branch>
minigit merge <branch>
```

It includes multiple branch refs, detached checkout, shared-ancestor discovery,
fast-forward merging, whole-file three-way merging, conflict markers, and
two-parent merge commits.

## Current Limitations

- There is no command for staging file deletions.
- Checkout, switch, and merge do not protect unsaved working changes from being
  overwritten.
- Three-way merge works at whole-file granularity. Different edits to the same
  file conflict even when they affect separate lines.
- There is no merge-abort command.
- Concurrent MiniGit processes are not protected by repository lock files.
- Symlinks are rejected rather than stored as tracked objects.
- Objects use readable JSON for learning, not the binary Git object format.
- Push, pull, clone, users, and permissions belong to later phases.

## Roadmap

1. Local MiniGit: complete.
2. Branching and merging: complete.
3. Collaboration server with authentication and repository permissions.
4. Pull requests, comments, history API, and possibly a web interface.
5. Security hardening and fuzzing.
