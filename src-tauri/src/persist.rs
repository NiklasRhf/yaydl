use std::{
    fmt, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{de::DeserializeOwned, Serialize};
use tracing::warn;
use yaydl_shared::YaydlError;

#[derive(Debug, Clone, PartialEq)]
pub struct PersistError {
    pub path: PathBuf,
    pub message: String,
}

impl PersistError {
    pub fn new(path: &Path, message: impl Into<String>) -> Self {
        Self {
            path: path.to_path_buf(),
            message: message.into(),
        }
    }
}

impl fmt::Display for PersistError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}

impl std::error::Error for PersistError {}

impl From<PersistError> for YaydlError {
    fn from(e: PersistError) -> Self {
        YaydlError::Persist(e.to_string())
    }
}

/// `Ok(None)` only when the file does not exist. Anything unreadable or
/// unparsable is an error the caller has to handle, usually via [`quarantine`].
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, PersistError> {
    let Some(raw) = read_optional(path)? else {
        return Ok(None);
    };
    serde_json::from_str(&raw)
        .map(Some)
        .map_err(|e| PersistError::new(path, format!("invalid JSON: {e}")))
}

pub fn read_toml<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, PersistError> {
    let Some(raw) = read_optional(path)? else {
        return Ok(None);
    };
    toml::from_str(&raw)
        .map(Some)
        .map_err(|e| PersistError::new(path, format!("invalid TOML: {e}")))
}

pub fn read_optional(path: &Path) -> Result<Option<String>, PersistError> {
    match fs::read_to_string(path) {
        Ok(raw) => Ok(Some(raw)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(PersistError::new(path, format!("reading failed: {e}"))),
    }
}

pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), PersistError> {
    let serialized = serde_json::to_string_pretty(value)
        .map_err(|e| PersistError::new(path, format!("serializing failed: {e}")))?;
    write_atomic(path, serialized.as_bytes())
}

pub fn write_toml_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), PersistError> {
    let serialized = toml::to_string(value)
        .map_err(|e| PersistError::new(path, format!("serializing failed: {e}")))?;
    write_atomic(path, serialized.as_bytes())
}

/// Writes to a sibling temp file and renames it over the target, so a crash
/// mid-write leaves either the old or the new file, never a truncated one.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), PersistError> {
    let parent = path
        .parent()
        .ok_or_else(|| PersistError::new(path, "path has no parent directory"))?;
    fs::create_dir_all(parent).map_err(|e| {
        PersistError::new(path, format!("creating {} failed: {e}", parent.display()))
    })?;
    let file_name = path
        .file_name()
        .ok_or_else(|| PersistError::new(path, "path has no file name"))?
        .to_string_lossy();
    let tmp = parent.join(format!(".{file_name}.tmp"));
    let mut file = fs::File::create(&tmp)
        .map_err(|e| PersistError::new(path, format!("creating {} failed: {e}", tmp.display())))?;
    file.write_all(contents)
        .and_then(|_| file.sync_all())
        .map_err(|e| PersistError::new(path, format!("writing {} failed: {e}", tmp.display())))?;
    drop(file);
    fs::rename(&tmp, path).map_err(|e| {
        PersistError::new(
            path,
            format!("renaming {} into place failed: {e}", tmp.display()),
        )
    })
}

/// Moves an unreadable file aside as `<name>.invalid-<unix seconds>` so the app
/// can start with defaults without destroying what the user had. Callers must
/// tell the user where it went.
pub fn quarantine(path: &Path) -> Result<PathBuf, PersistError> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| PersistError::new(path, format!("system clock before unix epoch: {e}")))?
        .as_secs();
    let file_name = path
        .file_name()
        .ok_or_else(|| PersistError::new(path, "path has no file name"))?
        .to_string_lossy();
    let target = path.with_file_name(format!("{file_name}.invalid-{secs}"));
    fs::rename(path, &target).map_err(|e| {
        PersistError::new(
            path,
            format!("moving aside to {} failed: {e}", target.display()),
        )
    })?;
    warn!(from = %path.display(), to = %target.display(), "quarantined an invalid file");
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Doc {
        version: u32,
        name: String,
    }

    #[test]
    fn missing_file_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let got: Option<Doc> = read_json(&dir.path().join("nope.json")).unwrap();
        assert_eq!(got, None);
    }

    #[test]
    fn atomic_write_round_trips_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("doc.json");
        let doc = Doc {
            version: 1,
            name: "a".into(),
        };
        write_json_atomic(&path, &doc).unwrap();
        assert_eq!(read_json::<Doc>(&path).unwrap(), Some(doc));
        let names: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("doc.json")]);
    }

    #[test]
    fn invalid_file_is_an_error_and_quarantine_moves_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.json");
        fs::write(&path, "{not json").unwrap();
        assert!(read_json::<Doc>(&path).is_err());
        let moved = quarantine(&path).unwrap();
        assert!(!path.exists());
        assert_eq!(fs::read_to_string(moved).unwrap(), "{not json");
    }
}
