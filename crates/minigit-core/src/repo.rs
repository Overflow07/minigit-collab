//! Repository operations.
//!
//! This is intentionally a skeleton. Each method maps to one CLI command or
//! one Git concept we will build later.

use crate::object::{hash_bytes, Blob, Commit, Object, ObjectHash, Tree, TreeEntry};
use std::collections::{BTreeMap, BTreeSet};
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
    #[error("repository has no commits yet")]
    NoCommitsYet,
    #[error("branch already exists: {0}")]
    BranchAlreadyExists(String),
    #[error("invalid branch name: {0}")]
    InvalidBranchName(String),
    #[error("HEAD is detached")]
    DetachedHead,
    #[error("HEAD is empty or malformed")]
    InvalidHead,
    #[error("branch not found: {0}")]
    BranchNotFound(String),
    #[error("branches do not share a common ancestor")]
    NoCommonAncestor,
    #[error("cannot commit while merge conflicts remain")]
    UnresolvedConflicts,
    #[error("a merge is already in progress")]
    MergeInProgress,
}

pub type Result<T> = std::result::Result<T, RepoError>;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepositoryStatus {
    pub staged: Vec<String>,
    pub modified: Vec<String>,
    pub untracked: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    AlreadyUpToDate,
    FastForward(ObjectHash),
    Merged(ObjectHash),
    Conflicts(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EntryMerge {
    Resolved(Option<ObjectHash>),
    Conflict,
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

    fn validate_branch_name(name: &str) -> Result<()> {
        let is_valid = !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');

        if !is_valid {
            return Err(RepoError::InvalidBranchName(name.to_string()));
        }

        Ok(())
    }

    fn current_branch_ref(&self) -> Result<String> {
        let head = fs::read_to_string(self.git_dir.join("HEAD"))?;

        Ok(head
            .trim()
            .strip_prefix("ref: ")
            .ok_or(RepoError::DetachedHead)?
            .to_string())
    }

    fn head_commit(&self) -> Result<Option<ObjectHash>> {
        let head = fs::read_to_string(self.git_dir.join("HEAD"))?;
        let trimmed = head.trim();

        if let Some(branch_ref) = trimmed.strip_prefix("ref: ") {
            let content = fs::read_to_string(self.git_dir.join(branch_ref))?;
            let commit_hash = content.trim();

            if commit_hash.is_empty() {
                Ok(None)
            } else {
                Ok(Some(commit_hash.to_string()))
            }
        } else if trimmed.is_empty() {
            Err(RepoError::InvalidHead)
        } else {
            Ok(Some(trimmed.to_string()))
        }
    }

    fn read_merge_head(&self) -> Result<Option<ObjectHash>> {
        let path = self.git_dir.join("MERGE_HEAD");

        match fs::read_to_string(path) {
            Ok(content) => {
                let hash = content.trim();
                Self::validate_object_hash(hash)?;

                Ok(Some(hash.to_string()))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn ensure_no_merge_in_progress(&self) -> Result<()> {
        if self.read_merge_head()?.is_some() {
            return Err(RepoError::MergeInProgress);
        }

        Ok(())
    }

    fn read_merge_conflicts(&self) -> Result<Vec<String>> {
        let path = self.git_dir.join("MERGE_CONFLICTS");

        match fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }

    fn update_current_branch(&self, commit_hash: &str) -> Result<()> {
        let branch_ref = self.current_branch_ref()?;

        fs::write(self.git_dir.join(branch_ref), format!("{commit_hash}\n"))?;

        Ok(())
    }

    pub fn create_branch(&self, name: &str) -> Result<()> {
        Self::validate_branch_name(name)?;

        let branch_path = self.git_dir.join("refs").join("heads").join(name);

        if branch_path.exists() {
            return Err(RepoError::BranchAlreadyExists(name.to_string()));
        }

        let commit_hash = self.head_commit()?.ok_or(RepoError::NoCommitsYet)?;
        fs::write(branch_path, format!("{commit_hash}\n"))?;

        Ok(())
    }

    fn branch_commit(&self, name: &str) -> Result<ObjectHash> {
        Self::validate_branch_name(name)?;

        let branch_path = self.git_dir.join("refs").join("heads").join(name);

        if !branch_path.is_file() {
            return Err(RepoError::BranchNotFound(name.to_string()));
        }

        let content = fs::read_to_string(branch_path)?;
        let commit_hash = content.trim();

        if commit_hash.is_empty() {
            return Err(RepoError::NoCommitsYet);
        }

        Self::validate_object_hash(commit_hash)?;

        Ok(commit_hash.to_string())
    }

    pub fn switch_branch(&self, name: &str) -> Result<()> {
        self.ensure_no_merge_in_progress()?;

        let commit_hash = self.branch_commit(name)?;
        self.restore_commit(&commit_hash)?;

        let branch_ref = format!("refs/heads/{name}");
        fs::write(self.git_dir.join("HEAD"), format!("ref: {branch_ref}\n"))?;

        Ok(())
    }

    pub fn merge(&self, branch: &str) -> Result<MergeOutcome> {
        self.ensure_no_merge_in_progress()?;
        self.current_branch_ref()?;

        let current_hash = self.head_commit()?.ok_or(RepoError::NoCommitsYet)?;
        let target_hash = self.branch_commit(branch)?;

        let ancestor = self
            .find_common_ancestor(&current_hash, &target_hash)?
            .ok_or(RepoError::NoCommonAncestor)?;

        if ancestor == target_hash {
            return Ok(MergeOutcome::AlreadyUpToDate);
        }

        if ancestor == current_hash {
            self.restore_commit(&target_hash)?;
            self.update_current_branch(&target_hash)?;

            return Ok(MergeOutcome::FastForward(target_hash));
        }

        let ancestor_entries = self.entries_for_commit(&ancestor)?;
        let current_entries = self.entries_for_commit(&current_hash)?;
        let target_entries = self.entries_for_commit(&target_hash)?;

        let (merged_entries, conflicts) =
            Self::merge_entries(&ancestor_entries, &current_entries, &target_entries);

        if !conflicts.is_empty() {
            self.restore_entries(&merged_entries)?;

            for path in &conflicts {
                self.write_conflict_file(
                    path,
                    current_entries.get(path),
                    target_entries.get(path),
                    branch,
                )?;
            }

            fs::write(self.git_dir.join("MERGE_HEAD"), format!("{target_hash}\n"))?;

            fs::write(
                self.git_dir.join("MERGE_CONFLICTS"),
                serde_json::to_vec_pretty(&conflicts)?,
            )?;

            return Ok(MergeOutcome::Conflicts(conflicts));
        }

        let merge_hash = self.write_commit_object(
            &merged_entries,
            vec![current_hash, target_hash],
            format!("Merge branch '{branch}'"),
        )?;

        self.restore_commit(&merge_hash)?;
        self.update_current_branch(&merge_hash)?;

        Ok(MergeOutcome::Merged(merge_hash))
    }

    pub fn add(&self, path: impl AsRef<Path>) -> Result<ObjectHash> {
        let path = path.as_ref();

        Self::validate_repo_path(path)?;
        self.validate_no_symlinks(path)?;

        let bytes = fs::read(self.worktree.join(path))?;

        let object = Object::Blob(Blob { bytes });
        let hash = self.write_object(&object)?;

        let path_string = path.to_string_lossy().to_string();

        let mut index = self.read_index()?;
        index.entries.insert(path_string.clone(), hash.clone());
        self.write_index(&index)?;

        let mut conflicts = self.read_merge_conflicts()?;
        let previous_len = conflicts.len();

        conflicts.retain(|conflict| conflict != &path_string);

        if conflicts.len() != previous_len {
            fs::write(
                self.git_dir.join("MERGE_CONFLICTS"),
                serde_json::to_vec_pretty(&conflicts)?,
            )?;
        }

        Ok(hash)
    }

    fn write_commit_object(
        &self,
        entries: &BTreeMap<String, ObjectHash>,
        parents: Vec<ObjectHash>,
        message: String,
    ) -> Result<ObjectHash> {
        let mut tree_entries = Vec::new();

        for (path, blob) in entries {
            tree_entries.push(TreeEntry {
                path: path.clone(),
                blob: blob.clone(),
            });
        }

        let tree = Object::Tree(Tree {
            entries: tree_entries,
        });

        let tree_hash = self.write_object(&tree)?;

        let commit = Object::Commit(Commit {
            tree: tree_hash,
            parents,
            message,
            timestamp_secs: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        });

        self.write_object(&commit)
    }

    pub fn commit(&self, message: impl Into<String>) -> Result<ObjectHash> {
        self.current_branch_ref()?;

        let conflicts = self.read_merge_conflicts()?;

        if !conflicts.is_empty() {
            return Err(RepoError::UnresolvedConflicts);
        }

        let merge_parent = self.read_merge_head()?;
        let completing_merge = merge_parent.is_some();

        let index = self.read_index()?;
        let committed = self.committed_entries()?;

        if index.entries == committed && !completing_merge {
            return Err(RepoError::NothingToCommit);
        }

        let mut parents = match self.head_commit()? {
            Some(hash) => vec![hash],
            None => Vec::new(),
        };

        if let Some(hash) = merge_parent {
            parents.push(hash);
        }

        let commit_hash = self.write_commit_object(&index.entries, parents, message.into())?;

        self.update_current_branch(&commit_hash)?;

        if completing_merge {
            for name in ["MERGE_HEAD", "MERGE_CONFLICTS"] {
                match fs::remove_file(self.git_dir.join(name)) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }

        Ok(commit_hash)
    }

    pub fn log(&self) -> Result<Vec<(ObjectHash, Commit)>> {
        let mut commits = Vec::new();
        let mut current = self.head_commit()?;

        while let Some(hash) = current {
            let object = self.read_object(&hash)?;

            match object {
                Object::Commit(commit) => {
                    current = commit.parents.first().cloned();
                    commits.push((hash, commit));
                }
                _ => return Err(RepoError::ExpectedCommit),
            }
        }

        Ok(commits)
    }

    fn committed_entries(&self) -> Result<BTreeMap<String, ObjectHash>> {
        match self.head_commit()? {
            Some(hash) => self.entries_for_commit(&hash),
            None => Ok(BTreeMap::new()),
        }
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

    fn restore_entries(&self, entries: &BTreeMap<String, ObjectHash>) -> Result<()> {
        let current_index = self.read_index()?;
        let target_index = Index {
            entries: entries.clone(),
        };

        for (path, blob_hash) in entries {
            let relative = Path::new(path);
            Self::validate_repo_path(relative)?;
            self.validate_no_symlinks(relative)?;

            let blob = match self.read_object(blob_hash)? {
                Object::Blob(blob) => blob,
                _ => return Err(RepoError::ExpectedBlob),
            };

            let destination = self.worktree.join(relative);

            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }

            fs::write(destination, blob.bytes)?;
        }

        for path in current_index.entries.keys() {
            if entries.contains_key(path) {
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

        Ok(())
    }

    fn restore_commit(&self, commit_hash: &str) -> Result<()> {
        let entries = self.entries_for_commit(commit_hash)?;
        self.restore_entries(&entries)
    }

    pub fn checkout(&self, commit_hash: &str) -> Result<()> {
        self.ensure_no_merge_in_progress()?;

        self.restore_commit(commit_hash)?;

        fs::write(self.git_dir.join("HEAD"), format!("{commit_hash}\n"))?;

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

    fn entries_for_commit(&self, commit_hash: &str) -> Result<BTreeMap<String, ObjectHash>> {
        let tree = self.tree_for_commit(commit_hash)?;
        let mut entries = BTreeMap::new();

        for entry in tree.entries {
            entries.insert(entry.path, entry.blob);
        }

        Ok(entries)
    }

    fn blob_bytes_for_merge(&self, hash: Option<&ObjectHash>) -> Result<Vec<u8>> {
        match hash {
            None => Ok(Vec::new()),
            Some(hash) => match self.read_object(hash)? {
                Object::Blob(blob) => Ok(blob.bytes),
                _ => Err(RepoError::ExpectedBlob),
            },
        }
    }

    fn conflict_bytes(current: &[u8], target: &[u8], target_branch: &str) -> Vec<u8> {
        let mut result = Vec::new();

        result.extend_from_slice(b"<<<<<<< HEAD\n");
        result.extend_from_slice(current);

        if !current.ends_with(b"\n") {
            result.push(b'\n');
        }

        result.extend_from_slice(b"=======\n");
        result.extend_from_slice(target);

        if !target.ends_with(b"\n") {
            result.push(b'\n');
        }

        result.extend_from_slice(format!(">>>>>>> {target_branch}\n").as_bytes());

        result
    }

    fn write_conflict_file(
        &self,
        path: &str,
        current_hash: Option<&ObjectHash>,
        target_hash: Option<&ObjectHash>,
        target_branch: &str,
    ) -> Result<()> {
        let relative = Path::new(path);

        Self::validate_repo_path(relative)?;
        self.validate_no_symlinks(relative)?;

        let current = self.blob_bytes_for_merge(current_hash)?;
        let target = self.blob_bytes_for_merge(target_hash)?;
        let contents = Self::conflict_bytes(&current, &target, target_branch);

        let destination = self.worktree.join(relative);

        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(destination, contents)?;

        Ok(())
    }

    fn merge_entry(
        ancestor: Option<&ObjectHash>,
        current: Option<&ObjectHash>,
        target: Option<&ObjectHash>,
    ) -> EntryMerge {
        if current == target {
            EntryMerge::Resolved(current.cloned())
        } else if current == ancestor {
            EntryMerge::Resolved(target.cloned())
        } else if target == ancestor {
            EntryMerge::Resolved(current.cloned())
        } else {
            EntryMerge::Conflict
        }
    }

    fn merge_entries(
        ancestor: &BTreeMap<String, ObjectHash>,
        current: &BTreeMap<String, ObjectHash>,
        target: &BTreeMap<String, ObjectHash>,
    ) -> (BTreeMap<String, ObjectHash>, Vec<String>) {
        let mut paths = BTreeSet::new();
        paths.extend(ancestor.keys().cloned());
        paths.extend(current.keys().cloned());
        paths.extend(target.keys().cloned());

        let mut merged = BTreeMap::new();
        let mut conflicts = Vec::new();

        for path in paths {
            match Self::merge_entry(ancestor.get(&path), current.get(&path), target.get(&path)) {
                EntryMerge::Resolved(Some(hash)) => {
                    merged.insert(path, hash);
                }
                EntryMerge::Resolved(None) => {}
                EntryMerge::Conflict => {
                    conflicts.push(path);
                }
            }
        }

        (merged, conflicts)
    }

    fn parents_of_commit(&self, commit_hash: &str) -> Result<Vec<ObjectHash>> {
        match self.read_object(commit_hash)? {
            Object::Commit(commit) => Ok(commit.parents),
            _ => Err(RepoError::ExpectedCommit),
        }
    }

    fn find_common_ancestor(
        &self,
        left_hash: &str,
        right_hash: &str,
    ) -> Result<Option<ObjectHash>> {
        let mut left_ancestors = BTreeSet::new();
        let mut left_to_visit = vec![left_hash.to_string()];

        while let Some(hash) = left_to_visit.pop() {
            if left_ancestors.insert(hash.clone()) {
                left_to_visit.extend(self.parents_of_commit(&hash)?);
            }
        }

        let mut right_visited = BTreeSet::new();
        let mut right_to_visit = vec![right_hash.to_string()];

        while let Some(hash) = right_to_visit.pop() {
            if left_ancestors.contains(&hash) {
                return Ok(Some(hash));
            }

            if right_visited.insert(hash.clone()) {
                right_to_visit.extend(self.parents_of_commit(&hash)?);
            }
        }

        Ok(None)
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
    fn merge_entry_follows_three_way_rules() {
        let ancestor = "ancestor".to_string();
        let current = "current".to_string();
        let target = "target".to_string();

        assert_eq!(
            Repository::merge_entry(Some(&ancestor), Some(&current), Some(&current)),
            EntryMerge::Resolved(Some(current.clone()))
        );

        assert_eq!(
            Repository::merge_entry(Some(&ancestor), Some(&ancestor), Some(&target)),
            EntryMerge::Resolved(Some(target.clone()))
        );

        assert_eq!(
            Repository::merge_entry(Some(&ancestor), Some(&current), Some(&ancestor)),
            EntryMerge::Resolved(Some(current.clone()))
        );

        assert_eq!(
            Repository::merge_entry(Some(&ancestor), Some(&current), Some(&target)),
            EntryMerge::Conflict
        );
    }

    #[test]
    fn merge_entries_combines_independent_changes() {
        let ancestor = BTreeMap::from([
            ("a.txt".to_string(), "a-old".to_string()),
            ("b.txt".to_string(), "b-old".to_string()),
        ]);

        let current = BTreeMap::from([
            ("a.txt".to_string(), "a-new".to_string()),
            ("b.txt".to_string(), "b-old".to_string()),
        ]);

        let target = BTreeMap::from([
            ("a.txt".to_string(), "a-old".to_string()),
            ("b.txt".to_string(), "b-new".to_string()),
        ]);

        let (merged, conflicts) = Repository::merge_entries(&ancestor, &current, &target);

        assert_eq!(merged.get("a.txt"), Some(&"a-new".to_string()));
        assert_eq!(merged.get("b.txt"), Some(&"b-new".to_string()));
        assert!(conflicts.is_empty());
    }

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
    fn create_branch_points_to_current_commit() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        fs::write(temp.path().join("hello.txt"), "hello").unwrap();
        repo.add("hello.txt").unwrap();
        let commit_hash = repo.commit("first").unwrap();

        repo.create_branch("feature").unwrap();

        let branch_contents =
            fs::read_to_string(repo.git_dir.join("refs").join("heads").join("feature")).unwrap();

        assert_eq!(branch_contents.trim(), commit_hash);
    }

    #[test]
    fn create_branch_requires_a_commit() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        let result = repo.create_branch("feature");

        assert!(matches!(result, Err(RepoError::NoCommitsYet)));
    }

    #[test]
    fn create_branch_rejects_duplicate_name() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        fs::write(temp.path().join("hello.txt"), "hello").unwrap();
        repo.add("hello.txt").unwrap();
        repo.commit("first").unwrap();

        repo.create_branch("feature").unwrap();
        let result = repo.create_branch("feature");

        assert!(matches!(result, Err(RepoError::BranchAlreadyExists(_))));
    }

    #[test]
    fn create_branch_rejects_invalid_name() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        let result = repo.create_branch("../feature");

        assert!(matches!(result, Err(RepoError::InvalidBranchName(_))));
    }

    #[test]
    fn switch_branch_restores_files_and_updates_head() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        let file_path = temp.path().join("hello.txt");

        fs::write(&file_path, "version one").unwrap();
        repo.add("hello.txt").unwrap();
        let first = repo.commit("first commit").unwrap();

        repo.create_branch("feature").unwrap();

        fs::write(&file_path, "version two").unwrap();
        repo.add("hello.txt").unwrap();
        let second = repo.commit("second commit").unwrap();

        repo.switch_branch("feature").unwrap();

        assert_eq!(fs::read_to_string(file_path).unwrap(), "version one");
        assert_eq!(repo.current_branch_ref().unwrap(), "refs/heads/feature");
        assert_eq!(repo.head_commit().unwrap(), Some(first));

        let main =
            fs::read_to_string(repo.git_dir.join("refs").join("heads").join("main")).unwrap();

        assert_eq!(main.trim(), second);
    }

    #[test]
    fn switch_branch_rejects_missing_branch() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        let result = repo.switch_branch("missing");

        assert!(matches!(result, Err(RepoError::BranchNotFound(_))));
    }

    #[test]
    fn find_common_ancestor_finds_branch_point() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        let file_path = temp.path().join("hello.txt");

        fs::write(&file_path, "base").unwrap();
        repo.add("hello.txt").unwrap();
        let base = repo.commit("base commit").unwrap();

        repo.create_branch("feature").unwrap();

        fs::write(&file_path, "main version").unwrap();
        repo.add("hello.txt").unwrap();
        let main_tip = repo.commit("main commit").unwrap();

        repo.switch_branch("feature").unwrap();

        fs::write(&file_path, "feature version").unwrap();
        repo.add("hello.txt").unwrap();
        let feature_tip = repo.commit("feature commit").unwrap();

        let ancestor = repo.find_common_ancestor(&main_tip, &feature_tip).unwrap();

        assert_eq!(ancestor, Some(base));
    }

    #[test]
    fn merge_fast_forwards_current_branch() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();
        let file_path = temp.path().join("hello.txt");

        fs::write(&file_path, "base").unwrap();
        repo.add("hello.txt").unwrap();
        repo.commit("base commit").unwrap();

        repo.create_branch("feature").unwrap();
        repo.switch_branch("feature").unwrap();

        fs::write(&file_path, "feature version").unwrap();
        repo.add("hello.txt").unwrap();
        let feature_tip = repo.commit("feature commit").unwrap();

        repo.switch_branch("main").unwrap();
        let outcome = repo.merge("feature").unwrap();

        assert_eq!(outcome, MergeOutcome::FastForward(feature_tip.clone()));
        assert_eq!(repo.head_commit().unwrap(), Some(feature_tip));
        assert_eq!(fs::read_to_string(file_path).unwrap(), "feature version");
    }

    #[test]
    fn merge_combines_independent_branch_changes() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        let a_path = temp.path().join("a.txt");
        let b_path = temp.path().join("b.txt");

        fs::write(&a_path, "a-old").unwrap();
        fs::write(&b_path, "b-old").unwrap();
        repo.add("a.txt").unwrap();
        repo.add("b.txt").unwrap();
        repo.commit("base").unwrap();

        repo.create_branch("feature").unwrap();

        fs::write(&a_path, "a-main").unwrap();
        repo.add("a.txt").unwrap();
        let main_tip = repo.commit("main change").unwrap();

        repo.switch_branch("feature").unwrap();
        fs::write(&b_path, "b-feature").unwrap();
        repo.add("b.txt").unwrap();
        let feature_tip = repo.commit("feature change").unwrap();

        repo.switch_branch("main").unwrap();
        let outcome = repo.merge("feature").unwrap();

        let merge_hash = match outcome {
            MergeOutcome::Merged(hash) => hash,
            _ => panic!("expected a three-way merge"),
        };

        assert_eq!(fs::read_to_string(a_path).unwrap(), "a-main");
        assert_eq!(fs::read_to_string(b_path).unwrap(), "b-feature");
        assert_eq!(repo.head_commit().unwrap(), Some(merge_hash.clone()));

        match repo.read_object(&merge_hash).unwrap() {
            Object::Commit(commit) => {
                assert_eq!(commit.parents, vec![main_tip, feature_tip]);
            }
            _ => panic!("expected commit object"),
        }
    }

    #[test]
    fn merge_conflict_can_be_resolved_and_committed() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();
        let file_path = temp.path().join("notes.txt");

        fs::write(&file_path, "base").unwrap();
        repo.add("notes.txt").unwrap();
        repo.commit("base").unwrap();

        repo.create_branch("feature").unwrap();

        fs::write(&file_path, "main version").unwrap();
        repo.add("notes.txt").unwrap();
        let main_tip = repo.commit("main change").unwrap();

        repo.switch_branch("feature").unwrap();
        fs::write(&file_path, "feature version").unwrap();
        repo.add("notes.txt").unwrap();
        let feature_tip = repo.commit("feature change").unwrap();

        repo.switch_branch("main").unwrap();
        let outcome = repo.merge("feature").unwrap();

        assert_eq!(
            outcome,
            MergeOutcome::Conflicts(vec!["notes.txt".to_string()])
        );
        assert_eq!(
            fs::read_to_string(&file_path).unwrap(),
            "<<<<<<< HEAD\nmain version\n=======\nfeature version\n>>>>>>> feature\n"
        );
        assert!(matches!(
            repo.commit("too early"),
            Err(RepoError::UnresolvedConflicts)
        ));
        assert!(matches!(
            repo.switch_branch("feature"),
            Err(RepoError::MergeInProgress)
        ));

        fs::write(&file_path, "resolved version").unwrap();
        repo.add("notes.txt").unwrap();
        let merge_hash = repo.commit("resolve merge").unwrap();

        match repo.read_object(&merge_hash).unwrap() {
            Object::Commit(commit) => {
                assert_eq!(commit.parents, vec![main_tip, feature_tip]);
            }
            _ => panic!("expected commit object"),
        }

        assert_eq!(repo.head_commit().unwrap(), Some(merge_hash));
        assert!(!repo.git_dir.join("MERGE_HEAD").exists());
        assert!(!repo.git_dir.join("MERGE_CONFLICTS").exists());
    }

    #[test]
    fn merge_reports_already_up_to_date() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();
        let file_path = temp.path().join("hello.txt");

        fs::write(&file_path, "base").unwrap();
        repo.add("hello.txt").unwrap();
        repo.commit("base commit").unwrap();

        repo.create_branch("feature").unwrap();

        fs::write(&file_path, "main is newer").unwrap();
        repo.add("hello.txt").unwrap();
        let main_tip = repo.commit("main commit").unwrap();

        let outcome = repo.merge("feature").unwrap();

        assert_eq!(outcome, MergeOutcome::AlreadyUpToDate);
        assert_eq!(repo.head_commit().unwrap(), Some(main_tip));
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
                assert!(commit.parents.is_empty());
            }

            _ => panic!("expected commit object"),
        }
    }

    #[test]
    fn commit_rejects_detached_head() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        fs::write(temp.path().join("hello.txt"), "hello").unwrap();
        repo.add("hello.txt").unwrap();
        let commit_hash = repo.commit("first").unwrap();

        repo.checkout(&commit_hash).unwrap();

        let result = repo.commit("detached commit");

        assert!(matches!(result, Err(RepoError::DetachedHead)));
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
    fn checkout_detaches_head_without_moving_main() {
        let temp = TempDir::new().unwrap();
        let repo = Repository::init(temp.path()).unwrap();

        fs::write(temp.path().join("hello.txt"), "first").unwrap();
        repo.add("hello.txt").unwrap();
        let first = repo.commit("first").unwrap();

        fs::write(temp.path().join("hello.txt"), "second").unwrap();
        repo.add("hello.txt").unwrap();
        let second = repo.commit("second").unwrap();

        repo.checkout(&first).unwrap();

        let head = fs::read_to_string(repo.git_dir.join("HEAD")).unwrap();
        let main =
            fs::read_to_string(repo.git_dir.join("refs").join("heads").join("main")).unwrap();

        assert_eq!(head.trim(), first);
        assert_eq!(main.trim(), second);
        assert!(matches!(
            repo.current_branch_ref(),
            Err(RepoError::DetachedHead)
        ));
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
