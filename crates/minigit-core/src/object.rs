//! MiniGit object model.
//!
//! Keep this file small for now. We will implement hashing and parsing together
//! in the first learning step.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub type ObjectHash = String;

pub fn hash_bytes(bytes: &[u8]) -> ObjectHash {
    let mut hasher = Sha256::new();
    hasher.update(bytes);

    hex::encode(hasher.finalize())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blob {
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeEntry {
    pub path: String,
    pub blob: ObjectHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tree {
    pub entries: Vec<TreeEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    pub tree: ObjectHash,
    pub parent: Option<ObjectHash>,
    pub message: String,
    pub timestamp_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum Object {
    Blob(Blob),
    Tree(Tree),
    Commit(Commit),
}

impl Object {
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec_pretty(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    pub fn hash(&self) -> Result<ObjectHash, serde_json::Error> {
        let bytes = self.to_bytes()?;

        Ok(hash_bytes(&bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_round_trips_through_json() {
        let object = Object::Blob(Blob {
            bytes: b"hello\n".to_vec(),
        });

        let bytes = object.to_bytes().unwrap();
        let decoded = Object::from_bytes(&bytes).unwrap();

        assert_eq!(object, decoded);
    }

    #[test]
    fn hash_is_stable_for_same_object() {
        let object1 = Object::Blob(Blob {
            bytes: b"hello\n".to_vec(),
        });

        let object2 = Object::Blob(Blob {
            bytes: b"hello\n".to_vec(),
        });

        assert_eq!(object1.hash().unwrap(), object2.hash().unwrap());
    }

    #[test]
    fn hash_changes_when_content_changes() {
        let object1 = Object::Blob(Blob {
            bytes: b"hello\n".to_vec(),
        });

        let object2 = Object::Blob(Blob {
            bytes: b"goodbye\n".to_vec(),
        });

        assert_ne!(object1.hash().unwrap(), object2.hash().unwrap());
    }
}
