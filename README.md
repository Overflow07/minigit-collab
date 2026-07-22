# MiniGit Collab

MiniGit Collab is an educational Git-like version control system written in
Rust. It is built to teach content-addressed storage, repository internals,
filesystem programming, error handling, and later collaboration and fuzzing.

## Project Status

Phase 1 is implemented: MiniGit can initialize a local repository, stage files,
create commits, inspect history and status, and restore a commit. It is a
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

## Phase 1 Commands

Run these commands inside the folder that you want MiniGit to manage:

```bash
minigit init
minigit add <file>
minigit commit -m "message"
minigit log
minigit status
minigit checkout <commit>
```

## How It Works

- `add` stores the file contents as a Blob and records its hash in the index.
- `commit` stores the index as a Tree, then stores a Commit pointing to that
  Tree and the previous Commit.
- `HEAD` points to `refs/heads/main`, which stores the latest Commit hash.
- `log` follows each Commit parent hash from newest to oldest.
- `checkout` reads a Commit, its Tree, and each Blob to restore working files.
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
```

## Phase 2 Plan

Phase 2 is not implemented yet. It will add:

```bash
minigit branch <name>
minigit switch <branch>
minigit merge <branch>
```

The main concepts will be multiple branch refs, switching `HEAD` between
branches, finding a shared parent commit, three-way merging, and writing
conflict markers when both branches change the same content differently.

## Current Limitations

- Only the `main` branch exists; branching and merging are Phase 2.
- There is no command for staging file deletions.
- Checkout does not yet protect unsaved working changes from being overwritten.
- Symlinks are rejected rather than stored as tracked objects.
- Objects use readable JSON for learning, not the binary Git object format.
- Push, pull, clone, users, and permissions belong to later phases.

## Roadmap

1. Local MiniGit: complete.
2. Branching and merging.
3. Collaboration server with authentication and repository permissions.
4. Pull requests, comments, history API, and possibly a web interface.
5. Security hardening and fuzzing.
