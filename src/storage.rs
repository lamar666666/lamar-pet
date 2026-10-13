use crate::focus::FocusCheckpoint;
use crate::service::Preferences;
use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Document {
    pub schema_version: u32,
    #[serde(default)]
    pub preferences: Preferences,
    #[serde(default)]
    pub checkpoint: FocusCheckpoint,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            schema_version: 1,
            preferences: Preferences::default(),
            checkpoint: FocusCheckpoint::default(),
        }
    }
}

impl Document {
    pub fn is_valid(&self) -> bool {
        self.schema_version == 1 && self.preferences.is_valid() && self.checkpoint.is_valid()
    }
}

pub struct Storage {
    root: PathBuf,
}

impl Storage {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    pub fn load(&self) -> Result<Document, String> {
        let path = self.root.join("focus.json");
        match read_bounded(&path) {
            Ok(bytes) => parse(&bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Document::default()),
            Err(_) => Err("Unable to read focus data; existing files are preserved".into()),
        }
    }
    pub fn save(&self, document: &Document) -> Result<(), String> {
        if !document.is_valid() {
            return Err("Focus data failed validation".into());
        }
        let bytes =
            serde_json::to_vec_pretty(document).map_err(|_| "Unable to encode focus data")?;
        fs::create_dir_all(&self.root).map_err(|_| "Unable to create focus data directory")?;
        let path = self.root.join("focus.json");
        match read_bounded(&path) {
            Ok(previous) => {
                // A file modified/corrupted outside this process must never be
                // replaced silently, even if startup previously loaded it.
                parse(&previous)?;
                atomic_write(&self.root.join("focus.json.bak"), &previous)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Unable to read existing focus data before save".into()),
        }
        atomic_write(&path, &bytes)
    }
}

fn parse(bytes: &[u8]) -> Result<Document, String> {
    if bytes.len() > 1_048_576 {
        return Err("Focus data exceeds the size limit".into());
    }
    let document: Document =
        serde_json::from_slice(bytes).map_err(|_| "Focus data is corrupt; original preserved")?;
    if !document.is_valid() {
        return Err("Focus data version or values are invalid; original preserved".into());
    }
    Ok(document)
}

fn read_bounded(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(1_048_577)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    let mut file =
        AtomicWriteFile::open(path).map_err(|_| "Unable to prepare atomic focus save")?;
    file.write_all(bytes)
        .map_err(|_| "Unable to write focus data")?;
    file.sync_all().map_err(|_| "Unable to flush focus data")?;
    file.commit()
        .map_err(|_| "Unable to replace focus data atomically".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn missing_document_defaults_and_previous_fields_read() {
        let dir = tempdir().unwrap();
        let store = Storage::new(dir.path().into());
        assert!(store.load().unwrap().is_valid());
        fs::write(dir.path().join("focus.json"), br#"{"schema_version":1}"#).unwrap();
        let doc = store.load().unwrap();
        assert_eq!(doc.preferences.scale_percent, 100);
        assert_eq!(doc.checkpoint.focus_minutes, 25);
    }

    #[test]
    fn corrupt_or_unknown_version_remains_untouched() {
        let dir = tempdir().unwrap();
        let store = Storage::new(dir.path().into());
        for bytes in [b"broken".as_slice(), br#"{"schema_version":2}"#] {
            fs::write(dir.path().join("focus.json"), bytes).unwrap();
            assert!(store.load().is_err());
            assert_eq!(fs::read(dir.path().join("focus.json")).unwrap(), bytes);
        }
    }

    #[test]
    fn save_roundtrips_and_backup_keeps_previous_good_document() {
        let dir = tempdir().unwrap();
        let store = Storage::new(dir.path().into());
        let first = Document::default();
        store.save(&first).unwrap();
        let mut second = first.clone();
        second.preferences.scale_percent = 125;
        store.save(&second).unwrap();
        assert_eq!(store.load().unwrap().preferences.scale_percent, 125);
        let backup: Document =
            serde_json::from_slice(&fs::read(dir.path().join("focus.json.bak")).unwrap()).unwrap();
        assert_eq!(backup.preferences.scale_percent, 100);
    }

    #[test]
    fn failed_backup_write_preserves_primary() {
        let dir = tempdir().unwrap();
        let store = Storage::new(dir.path().into());
        store.save(&Document::default()).unwrap();
        let previous = fs::read(dir.path().join("focus.json")).unwrap();
        fs::create_dir(dir.path().join("focus.json.bak")).unwrap();
        let mut doc = Document::default();
        doc.preferences.scale_percent = 125;
        assert!(store.save(&doc).is_err());
        assert_eq!(fs::read(dir.path().join("focus.json")).unwrap(), previous);
    }
}
