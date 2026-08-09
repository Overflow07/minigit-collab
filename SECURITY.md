# Security Notes

MiniGit Collab is an educational project, not a hardened replacement for Git.
Do not use it with untrusted repositories or important data yet.

## Current Protections

- Object names must be 64 hexadecimal SHA-256 characters.
- Object contents are hashed again when read and must match their filename.
- Malformed JSON objects return errors instead of causing normal-user panics.
- Repository paths reject absolute paths, `..`, empty paths, and `.minigit`.
- Add and checkout reject paths containing existing symbolic links.
- Checkout tests verify that a symlink cannot redirect writes outside the
  working tree.
- Branch names allow only ASCII letters, digits, `-`, and `_`.
- Merge conflict writes reuse repository path and symlink validation.
- Checkout, switch, and merge are blocked while conflict resolution is active.

## Known Limitations

- Symlink checks and filesystem writes are separate operations, leaving a
  time-of-check/time-of-use race if another process changes paths concurrently.
- Repository operations do not use lock files, so concurrent processes can race
  while updating refs, the index, or merge state.
- `HEAD` and branch-ref paths are not fully validated against malicious manual
  repository edits.
- Checkout can overwrite unsaved or untracked files and can make partial changes
  if a later operation fails.
- Switch and merge can also overwrite unsaved files or leave partial filesystem
  changes if a later operation fails.
- Object and index file sizes are not bounded, so hostile files could exhaust
  memory or disk space.
- Local repository metadata is trusted more than a network-imported repository
  should be.

## Hardening Log

- Added object-hash format validation to prevent object-path traversal.
- Added content-hash verification to detect modified or corrupted objects.
- Added lexical checkout path validation.
- Added symlink-component checks for add and checkout.
- Added branch-name validation and detached-HEAD commit rejection.
- Added path and symlink checks for conflict-marker writes.

## Planned Fuzzing

- Object parser
- Commit parser
- Tree parser
- Network request parser
- Malformed repository import
- Checkout and path traversal
