use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;

use eyre::Result;

struct FileStatus(u64);

impl FileStatus {
    fn get(path: &Path) -> Result<Self> {
        let md = fs::metadata(path)?;
        let size = md.len();
        let mtime = md.modified()?;
        let mut hasher = DefaultHasher::new();
        mtime.hash(&mut hasher);
        size.hash(&mut hasher);
        Ok(Self(hasher.finish()))
    }
}

#[test]
fn file_status_changes() {}
