//! Repository operations.
//!
//! This is intentionally a skeleton. Each method maps to one CLI command or
//! one Git concept we will build later.

use crate::object::{hash_bytes, Blob, Commit, Object, ObjectHash, Tree, TreeEntry};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use walkdir::WalkDir;

#[derive(Debug, Error)]
pub enum RepoError {
    #[error("not a MiniGit repository")]
    NotRepository,
    #[error("repository already exists")]
    AlreadyExists,
    #[error("path does not exist: {0}")]
    PathNotFound(PathBuf),
    #[error("nothing to commit")]
    NothingToCommit,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("directory traversal error: {0}")]
    WalkDir(#[from] walkdir::Error),
    #[error("system clock error: {0}")]
    SystemTime(#[from] std::time::SystemTimeError),
    #[error("expected a commit object")]
    ExpectedCommit,
    #[error("expected a tree object")]
    ExpectedTree,
    #[error("expected a blob object")]
    ExpectedBlob,
    #[error("unsafe path in tree: {0}")]
    UnsafePath(PathBuf),
    #[error("invalid object hash: {0}")]
    InvalidObjectHash(String),
    #[error("object is corrupted: {0}")]
    CorruptedObject(ObjectHash),
}

pub type Result<T> = std::result::Result<T, RepoError>;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepositoryStatus {
    pub staged: Vec<String>,
    pub modified: Vec<String>,
    pub untracked: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
struct Index {
    entries: BTreeMap<String, ObjectHash>,
}

#[derive(Debug, Clone)]
pub struct Repository {
    pub worktree: PathBuf,
    pub git_dir: PathBuf,
}

impl Repository {
    pub fn init(path: impl AsRef<Path>) -> Result<Self> {
        let worktree = path.as_ref().to_path_buf();
        let git_dir = worktree.join(".minigit");

        if git_dir.exists() {
            return Err(RepoError::AlreadyExists);
        }

        fs::create_dir(&git_dir)?;
        fs::create_dir(git_dir.join("objects"))?;
        fs::create_dir_all(git_dir.join("refs").join("heads"))?;

        fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n")?;
        fs::write(git_dir.join("refs").join("heads").join("main"), "")?;
        fs::write(
            git_dir.join("index.json"),
            serde_json::to_vec_pretty(&Index::default())?,
        )?;

        Ok(Self { worktree, git_dir })
    }

    pub fn discover(start: impl AsRef<Path>) -> Result<Self> {
        let mut current = start.as_ref();

        loop {
            let git_dir = current.join(".minigit");

            if git_dir.is_dir() {
                return Ok(Self {
                    worktree: current.to_path_buf(),
                    git_dir,
                });
            }

            current = current.parent().ok_or(RepoError::NotRepository)?;
        }
    }

    pub fn write_object(&self, object: &Object) -> Result<ObjectHash> {
        let hash = object.hash()?;
        let bytes = object.to_bytes()?;

        fs::write(self.git_dir.join("objects").join(&hash), bytes)?;

        Ok(hash)
    }

    fn validate_object_hash(hash: &str) -> Result<()> {
        let is_valid = hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit());

        if !is_valid {
            return Err(RepoError::InvalidObjectHash(hash.to_string()));
        }

        Ok(())
    }

    pub fn read_object(&self, hash: &str) -> Result<Object> {
        Self::validate_object_hash(hash)?;

        let bytes = fs::read(self.git_dir.join("objects").join(hash))?;

        let actual_hash = hash_bytes(&bytes);

        if actual_hash != hash {
            return Err(RepoError::CorruptedObject(hash.to_string()));
        }

        Object::from_bytes(&bytes).map_err(RepoError::from)
    }

    fn read_index(&self) -> Result<Index> {
        let bytes = fs::read(self.git_dir.join("index.json"))?;

        serde_json::from_slice(&bytes).map_err(RepoError::from)
    }

    fn write_index(&self, index: &Index) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(index)?;

        fs::write(self.git_dir.join("index.json"), bytes)?;

        Ok(())
    }

    fn current_branch_ref(&self) -> Result<String> {
        let head = fs::read_to_string(self.git_dir.join("HEAD"))?;

        Ok(head
            .trim()
            .strip_prefix("ref: ")
            .ok_or(RepoError::NotRepository)?
            .to_string())
    }

    fn head_commit(&self) -> Result<Option<ObjectHash>> {
        let branch_ref = self.current_branch_ref()?;
        let content = fs::read_to_string(self.git_dir.join(branch_ref))?;
        let trimmed = content.trim();

        if trimmed.is_empty() {
            Ok(None)
        } else {
            Ok(Some(trimmed.to_string()))
        }
    }

    fn update_current_branch(&self, commit_hash: &str) -> Result<()> {
        let branch_ref = self.current_branch_ref()?;

        fs::write(self.git_dir.join(branch_ref), format!("{commit_hash}\n"))?;

        Ok(())
    }

    pub fn add(&self, path: impl AsRef<Path>) -> Result<ObjectHash> {
        let path = path.as_ref();

        Self::validate_repo_path(path)?;
        self.validate_no_symlinks(path)?;

        let bytes = fs::read(self.worktree.join(path))?;

        let object = Object::Blob(Blob { bytes });
        let hash = self.write_object(&object)?;

        let mut index = self.read_index()?;
        index
            .entries
            .insert(path.to_string_lossy().to_string(), hash.clone());
        self.write_index(&index)?;

        Ok(hash)
    }

    pub fn commit(&self, message: impl Into<String>) -> Result<ObjectHash> {
        let index = self.read_index()?;

        let committed = self.committed_entries()?;

        if index.entries == committed {
            return Err(RepoError::NothingToCommit);
        }
        let entries = index
            .entries
            .iter()
            .map(|(path, blob)| TreeEntry {
                path: path.clone(),
                blob: blob.clone(),
            })
            .collect();

        let tree = Object::Tree(Tree { entries });
        let tree_hash = self.write_object(&tree)?;

        let commit = Object::Commit(Commit {
            tree: tree_hash,
            parent: self.head_commit()?,
            message: message.into(),
            timestamp_secs: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        });

        let commit_hash = self.write_object(&commit)?;
        self.update_current_branch(&commit_hash)?;

        Ok(commit_hash)
    }

    pub fn log(&self) -> Result<Vec<(ObjectHash, Commit)>> {
        let mut commits = Vec::new();
        let mut current = self.head_commit()?;

        while let Some(hash) = current {
            let object = self.read_object(&hash)?;

            match object {
                Object::Commit(commit) => {
                    current = commit.parent.clone();
                    commits.push((hash, commit));
                }
                _ => return Err(RepoError::ExpectedCommit),
            }
        }

        Ok(commits)
    }

    fn committed_entries(&self) -> Result<BTreeMap<String, ObjectHash>> {
        let commit_hash = match self.head_commit()? {
            Some(hash) => hash,
            None => return Ok(BTreeMap::new()),
        };

        let tree = self.tree_for_commit(&commit_hash)?;

        let mut entries = BTreeMap::new();

        for entry in tree.entries {
            entries.insert(entry.path, entry.blob);
        }

        Ok(entries)
    }

    pub fn status(&self) -> Result<RepositoryStatus> {
        let index = self.read_index()?;

        let committed = self.committed_entries()?;
        let mut staged = Vec::new();

        for (path, indexed_hash) in &index.entries {
            if committed.get(path) != Some(indexed_hash) {
                staged.push(path.clone());
            }
        }

        let mut modified = Vec::new();

        for (path, stored_hash) in &index.entries {
            let working_path = self.worktree.join(path);

            if !working_path.exists() {
                modified.push(path.clone());
                continue;
            }

            let bytes = fs::read(working_path)?;
            let object = Object::Blob(Blob { bytes });
            let current_hash = object.hash()?;

            if current_hash != *stored_hash {
                modified.push(path.clone());
            }
        }

        let mut untracked = Vec::new();

        for entry in WalkDir::new(&self.worktree)
            .into_iter()
            .filter_entry(|entry| entry.path() != self.git_dir.as_path())
        {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }

            let relative = entry
                .path()
                .strip_prefix(&self.worktree)
                .map_err(|_| RepoError::PathNotFound(entry.path().to_path_buf()))?;

            let path = relative.to_string_lossy().to_string();

            if !index.entries.contains_key(&path) {
                untracked.push(path);
            }
        }

        Ok(RepositoryStatus {
            staged,
            modified,
            untracked,
        })
    }

    pub fn checkout(&self, commit_hash: &str) -> Result<()> {
        let tree = self.tree_for_commit(commit_hash)?;

        let current_index = self.read_index()?;

        let mut target_index = Index::default();

        for entry in tree.entries {
            let relative = Path::new(&entry.path);
            Self::validate_repo_path(relative)?;
            self.validate_no_symlinks(relative)?;

            let blob = match self.read_object(&entry.blob)? {
                Object::Blob(blob) => blob,
                _ => return Err(RepoError::ExpectedBlob),
            };

            let destination = self.worktree.join(relative);

            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }

            fs::write(destination, blob.bytes)?;

            target_index.entries.insert(entry.path, entry.blob);
        }

        for path in current_index.entries.keys() {
            if target_index.entries.contains_key(path) {
                continue;
            }

            let relative = Path::new(path);
            Self::validate_repo_path(relative)?;
            self.validate_no_symlinks(relative)?;

            let destination = self.worktree.join(relative);

            match fs::remove_file(destination) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        self.write_index(&target_index)?;
        self.update_current_branch(commit_hash)?;

        Ok(())
    }

    fn tree_for_commit(&self, commit_hash: &str) -> Result<Tree> {
        let commit = match self.read_object(commit_hash)? {
            Object::Commit(commit) => commit,
            _ => return Err(RepoError::ExpectedCommit),
        };

        match self.read_object(&commit.tree)? {
            Object::Tree(tree) => Ok(tree),
            _ => Err(RepoError::ExpectedTree),
        }
    }

    fn validate_no_symlinks(&self, path: &Path) -> Result<()> {
        let mut current = self.worktree.clone();

        for component in path.components() {
            current.push(component.as_os_str());

            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(RepoError::UnsafePath(path.to_path_buf()));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => return Err(error.into()),
            }
        }

        Ok(())
    }

    fn validate_repo_path(path: &Path) -> Result<()> {
        let is_unsafe = path.as_os_str().is_empty()
            || path.starts_with(".minigit")
            || path.is_absolute()
            || !path
                .components()
                .all(|component| matches!(component, Component::Normal(_)));

        if is_unsafe {
            return Err(RepoError::UnsafePath(path.to_path_buf()));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn init_creates_minigit_directory_layout() {
        let temp = TempDir::new().unwrap();

        Repository::init(temp.path()).unwrap();

        assert!(temp.path().join(".minigit").is_dir());
        assert!(temp.path().join(".minigit").join("objects").is_dir());
        assert!(temp
            .path()
            .join(".minigit")
            .join("refs")
            .join("heads")
            .is_dir());
        assert!(temp.path().join(".minigit").join("HEAD").is_file());
        assert!(temp.path().join(".minigit").join("index.json").is_file());
    }

    #[test]
    fn init_fails_when_repository_already_exists() {
        let temp = TempDir::new().unwrap();

        Repository::init(temp.path()).unwrap();

        let result = Repository::init(temp.path());

        assert!(matches!(result, Err(RepoError::AlreadyExists)));
    }

    #[test]
    fn discover_finds_repository_from_nested_directory() {
        let temp = TempDir::new().unwrap();

        Repository::init(temp.path()).unwrap();

        let nested = temp.path().join("src").join("nested");
        fs::create_dir_all(&nested).unwrap();

        let repo = Repository::discover(&nested).unwrap();

        assert_eq!(repo.worktree, temp.path());
        assert_eq!(repo.git_dir, temp.path().join(".minigit"));
    }

    #[test]
    fn write_object_stores_object_by_hash() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        let object = Object::Blob(Blob {
            bytes: b"hello\n".to_vec(),
        });

        let hash = repo.write_object(&object).unwrap();

        assert!(temp
            .path()
            .join(".minigit")
            .join("objects")
            .join(hash)
            .is_file());
    }

    #[test]
    fn read_object_restores_written_object() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        let object = Object::Blob(Blob {
            bytes: b"hello\n".to_vec(),
        });

        let hash = repo.write_object(&object).unwrap();
        let restored = repo.read_object(&hash).unwrap();

        assert_eq!(restored, object);
    }

    #[test]
    fn add_stores_blob_and_updates_index() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        fs::write(temp.path().join("hello.txt"), "hello\n").unwrap();

        let hash = repo.add("hello.txt").unwrap();

        assert!(temp
            .path()
            .join(".minigit")
            .join("objects")
            .join(&hash)
            .is_file());

        let index = repo.read_index().unwrap();
        assert_eq!(index.entries.get("hello.txt"), Some(&hash));
    }
    #[test]
    fn status_lists_staged_files() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        fs::write(temp.path().join("hello.txt"), "hello\n").unwrap();
        repo.add("hello.txt").unwrap();

        let status = repo.status().unwrap();

        assert_eq!(status.staged, vec!["hello.txt".to_string()]);
        assert!(status.modified.is_empty());
        assert!(status.untracked.is_empty());
    }

    #[test]
    fn current_branch_ref_reads_head_pointer() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        assert_eq!(repo.current_branch_ref().unwrap(), "refs/heads/main");
    }

    #[test]
    fn head_commit_is_none_before_first_commit() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        assert_eq!(repo.head_commit().unwrap(), None);
    }

    #[test]
    fn update_current_branch_changes_head_commit() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        repo.update_current_branch("abc123").unwrap();
        assert_eq!(repo.head_commit().unwrap(), Some("abc123".to_string()));
    }

    #[test]
    fn commit_creates_commit_and_updates_branch() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        fs::write(temp.path().join("hello.txt"), "hello\n").unwrap();
        repo.add("hello.txt").unwrap();

        let commit_hash = repo.commit("first commit").unwrap();

        assert_eq!(repo.head_commit().unwrap(), Some(commit_hash.clone()));

        let object = repo.read_object(&commit_hash).unwrap();

        match object {
            Object::Commit(commit) => {
                assert_eq!(commit.message, "first commit");
                assert_eq!(commit.parent, None);
            }

            _ => panic!("expected commit object"),
        }
    }

    #[test]
    fn commit_rejects_when_nothing_changed() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        fs::write(temp.path().join("hello.txt"), "hello").unwrap();
        repo.add("hello.txt").unwrap();
        repo.commit("first commit").unwrap();

        let result = repo.commit("duplicate commit");

        assert!(matches!(result, Err(RepoError::NothingToCommit)));
    }

    #[test]
    fn log_returns_commits_newest_first() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        fs::write(temp.path().join("hello.txt"), "one\n").unwrap();
        repo.add("hello.txt").unwrap();
        let first = repo.commit("first").unwrap();

        fs::write(temp.path().join("hello.txt"), "two\n").unwrap();
        repo.add("hello.txt").unwrap();

        let second = repo.commit("second").unwrap();
        let log = repo.log().unwrap();

        assert_eq!(log.len(), 2);
        assert_eq!(log[0].0, second);
        assert_eq!(log[0].1.message, "second");
        assert_eq!(log[1].0, first);
        assert_eq!(log[1].1.message, "first");
    }

    #[test]
    fn status_detects_modified_file() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        fs::write(temp.path().join("hello.txt"), "original").unwrap();
        repo.add("hello.txt").unwrap();

        fs::write(temp.path().join("hello.txt"), "changed").unwrap();

        let status = repo.status().unwrap();

        assert_eq!(status.modified, vec!["hello.txt".to_string()]);
    }

    #[test]
    fn status_detectd_untracked_file() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        fs::write(temp.path().join("notes.txt"), "not added").unwrap();

        let status = repo.status().unwrap();

        assert_eq!(status.untracked, vec!["notes.txt".to_string()]);
    }

    #[test]
    fn status_detects_deleted_file() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        let file_path = temp.path().join("hello.txt");

        fs::write(&file_path, "hello").unwrap();
        repo.add("hello.txt").unwrap();
        fs::remove_file(file_path).unwrap();

        let status = repo.status().unwrap();

        assert_eq!(status.modified, vec!["hello.txt".to_string()]);
    }

    #[test]
    fn status_has_no_staged_files_after_commit() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        fs::write(temp.path().join("hello.txt"), "hello").unwrap();
        repo.add("hello.txt").unwrap();
        repo.commit("first commit").unwrap();

        let status = repo.status().unwrap();

        assert!(status.staged.is_empty());
    }

    #[test]
    fn checkout_restores_file_contents() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        let file_path = temp.path().join("hello.txt");

        fs::write(&file_path, "original").unwrap();
        repo.add("hello.txt").unwrap();
        let commit_hash = repo.commit("first commit").unwrap();

        fs::write(&file_path, "changed").unwrap();

        repo.checkout(&commit_hash).unwrap();

        assert_eq!(fs::read_to_string(file_path).unwrap(), "original");
    }

    #[test]
    fn checkout_removes_tracked_files_missing_from_target() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        fs::write(temp.path().join("hello.txt"), "hello").unwrap();
        repo.add("hello.txt").unwrap();
        let first_commit = repo.commit("first").unwrap();

        let extra_path = temp.path().join("extra.txt");
        fs::write(&extra_path, "extra").unwrap();
        repo.add("extra.txt").unwrap();
        repo.commit("second").unwrap();

        repo.checkout(&first_commit).unwrap();

        assert!(!extra_path.exists());
    }

    #[test]
    fn add_rejects_parent_directory_path() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        let result = repo.add("../secret.txt");

        assert!(matches!(result, Err(RepoError::UnsafePath(_))));
    }

    #[test]
    fn read_object_rejects_invalid_hash() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        let result = repo.read_object("../../secret");
        assert!(matches!(result, Err(RepoError::InvalidObjectHash(_))));
    }

    #[test]
    fn read_object_rejects_corrupted_contents() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        let original = Object::Blob(Blob {
            bytes: b"hello".to_vec(),
        });

        let hash = repo.write_object(&original).unwrap();
        let changed = Object::Blob(Blob {
            bytes: b"goodbye".to_vec(),
        });

        fs::write(
            repo.git_dir.join("objects").join(&hash),
            changed.to_bytes().unwrap(),
        )
        .unwrap();

        let result = repo.read_object(&hash);

        assert!(matches!(result, Err(RepoError::CorruptedObject(_))));
    }

    #[cfg(unix)]
    #[test]
    fn checkout_rejects_symlink_path() {
        let temp = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        let folder = temp.path().join("folder");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("hello.txt"), "hello").unwrap();

        repo.add("folder/hello.txt").unwrap();
        let commit_hash = repo.commit("first").unwrap();

        fs::remove_file(folder.join("hello.txt")).unwrap();
        fs::remove_dir(&folder).unwrap();
        std::os::unix::fs::symlink(outside.path(), &folder).unwrap();

        let result = repo.checkout(&commit_hash);

        assert!(matches!(result, Err(RepoError::UnsafePath(_))));
        assert!(!outside.path().join("hello.txt").exists());
    }
}
