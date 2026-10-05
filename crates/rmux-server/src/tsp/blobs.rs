use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};
pub const BLOB_LIMIT: usize = 16 * 1024 * 1024;
pub const CACHE_BUDGET: usize = 256 * 1024 * 1024;
#[derive(Debug)]
pub struct Blob {
    pub bytes: Arc<[u8]>,
    pub mime: Option<String>,
    references: usize,
    touched: u64,
}
#[derive(Debug, Default)]
pub struct TspBlobStore {
    blobs: HashMap<String, Blob>,
    owners: HashMap<String, BTreeSet<String>>,
    clock: u64,
    bytes: usize,
}
impl TspBlobStore {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn insert(&mut self, id: &str, mime: Option<&str>, body: &[u8]) -> Result<String, String> {
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid SHA-256 id".into());
        }
        if body.len() > BLOB_LIMIT.div_ceil(3) * 4 {
            return Err("blob size limit".into());
        }
        let bytes = STANDARD
            .decode(body)
            .map_err(|_| "invalid standard base64")?;
        if bytes.len() > BLOB_LIMIT {
            return Err("blob size limit".into());
        }
        let digest = format!("{:x}", Sha256::digest(&bytes));
        if digest != id.to_ascii_lowercase() {
            return Err("blob digest mismatch".into());
        }
        self.clock += 1;
        if let Some(b) = self.blobs.get_mut(&digest) {
            b.touched = self.clock;
            return Ok(digest);
        }
        let references = self.owners.values().filter(|s| s.contains(&digest)).count();
        self.bytes += bytes.len();
        self.blobs.insert(
            digest.clone(),
            Blob {
                bytes: bytes.into(),
                mime: mime.map(str::to_owned),
                references,
                touched: self.clock,
            },
        );
        self.evict_to(CACHE_BUDGET);
        Ok(digest)
    }
    pub fn set_references(&mut self, owner: &str, ids: BTreeSet<String>) {
        let old = self
            .owners
            .insert(owner.into(), ids.clone())
            .unwrap_or_default();
        for id in old.difference(&ids) {
            if let Some(b) = self.blobs.get_mut(id) {
                b.references = b.references.saturating_sub(1);
            }
        }
        for id in ids.difference(&old) {
            if let Some(b) = self.blobs.get_mut(id) {
                b.references += 1;
            }
        }
        self.evict_to(CACHE_BUDGET);
    }
    pub fn remove_owner(&mut self, owner: &str) {
        self.set_references(owner, BTreeSet::new());
        self.owners.remove(owner);
    }
    pub fn get(&mut self, id: &str) -> Option<&Blob> {
        self.clock += 1;
        let b = self.blobs.get_mut(&id.to_ascii_lowercase())?;
        b.touched = self.clock;
        Some(b)
    }
    pub fn peek(&self, id: &str) -> Option<&Blob> {
        self.blobs.get(&id.to_ascii_lowercase())
    }
    pub fn have(&self, ids: &[String]) -> Vec<String> {
        ids.iter()
            .filter(|id| self.blobs.contains_key(&id.to_ascii_lowercase()))
            .cloned()
            .collect()
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn evict_to(&mut self, budget: usize) {
        while self.bytes > budget {
            let victim = self
                .blobs
                .iter()
                .filter(|(_, b)| b.references == 0)
                .min_by_key(|(_, b)| b.touched)
                .map(|(id, _)| id.clone());
            let Some(id) = victim else { break };
            self.bytes -= self.blobs.remove(&id).unwrap().bytes.len();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verifies_and_pins() {
        let mut s = TspBlobStore::new();
        let id = format!("{:x}", Sha256::digest(b"hello"));
        assert!(s.insert(&id, None, b"aGVsbG8=").is_ok());
        assert!(s.insert(&id, None, b"aGVsbG9=").is_err());
        assert!(s.insert(&"0".repeat(64), None, b"aGVsbG8=").is_err());
        s.set_references("sf", BTreeSet::from([id.clone()]));
        s.evict_to(0);
        assert_eq!(s.bytes(), 5);
        s.remove_owner("sf");
        s.evict_to(0);
        assert!(s.have(&[id]).is_empty());
    }
}
