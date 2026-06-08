// Plumbed into ChatSession::start in Phase 2.7 / 6.x. Until then the
// struct surface looks dead from the bin crate.
#![allow(dead_code)]

//! Per-project claude session id persistence.
//!
//! Spec: `docs/chat.md` 5. The store is a single JSON file at
//! `<data_dir>/sessions.json` keyed by absolute project root, holding the
//! last claude session id mdpilot saw plus light diagnostics.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

const STORE_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionStore {
    pub version: u32,
    #[serde(default)]
    pub entries: BTreeMap<String, SessionEntry>,
    /// Per-session "last preview file" so that resuming a session
    /// (F-11 auto-resume or the history picker in Phase 9.19) also
    /// reopens whichever markdown source the user was looking at.
    /// Keyed by session-id (UUID string) — independent of `entries`
    /// so the history picker can resume sessions that are no longer
    /// the project's "current" one.
    ///
    /// `#[serde(default)]` keeps old `sessions.json` files (written
    /// before this field existed) loading cleanly.
    #[serde(default)]
    pub session_previews: BTreeMap<String, PathBuf>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct SessionEntry {
    pub session_id: String,
    pub claude_version: String,
    pub last_used: DateTime<Utc>,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self {
            version: STORE_VERSION,
            entries: BTreeMap::new(),
            session_previews: BTreeMap::new(),
        }
    }
}

impl SessionStore {
    /// Read the store from disk, returning `Default` on any error (missing
    /// file, parse failure, version mismatch). Errors are logged via
    /// tracing — losing a stale store is preferable to refusing to launch.
    pub fn load_or_default(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(contents) => match serde_json::from_str::<Self>(&contents) {
                Ok(store) if store.version == STORE_VERSION => store,
                Ok(other) => {
                    tracing::warn!(
                        version = other.version,
                        expected = STORE_VERSION,
                        "session store version mismatch; starting fresh",
                    );
                    Self::default()
                }
                Err(err) => {
                    tracing::warn!("could not parse session store: {err}; starting fresh");
                    Self::default()
                }
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(err) => {
                tracing::warn!("could not read session store: {err}; starting fresh");
                Self::default()
            }
        }
    }

    /// Persist the store atomically: write to `<path>.tmp` then rename onto
    /// the final path. Parent directories are created if missing.
    pub fn save_atomic(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp_path = temp_path_for(path);
        {
            let mut file = fs::File::create(&temp_path)?;
            let json = serde_json::to_string_pretty(self)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
        }
        fs::rename(&temp_path, path)?;
        Ok(())
    }

    pub fn get(&self, project_root: &Path) -> Option<&SessionEntry> {
        self.entries.get(&key_for(project_root))
    }

    pub fn upsert(
        &mut self,
        project_root: &Path,
        session_id: String,
        claude_version: String,
    ) -> &SessionEntry {
        let key = key_for(project_root);
        self.entries.insert(
            key.clone(),
            SessionEntry {
                session_id,
                claude_version,
                last_used: Utc::now(),
            },
        );
        self.entries.get(&key).expect("just inserted")
    }

    /// Remember the markdown preview path the user had open when
    /// they were last in `session_id`. Pass `None` to forget (e.g.
    /// preview was cleared back to `Empty`).
    pub fn set_preview(&mut self, session_id: &str, path: Option<&Path>) {
        match path {
            Some(p) => {
                self.session_previews
                    .insert(session_id.to_string(), p.to_path_buf());
            }
            None => {
                self.session_previews.remove(session_id);
            }
        }
    }

    /// Look up the last preview path persisted for `session_id`.
    /// Returns `None` when the session was never persisted with a
    /// non-empty preview, or when the store predates this field.
    pub fn get_preview(&self, session_id: &str) -> Option<&Path> {
        self.session_previews.get(session_id).map(|p| p.as_path())
    }
}

fn key_for(project_root: &Path) -> String {
    project_root.to_string_lossy().into_owned()
}

fn temp_path_for(path: &Path) -> PathBuf {
    let mut file_name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from("sessions"));
    file_name.push(".tmp");
    let mut tmp = path.to_path_buf();
    tmp.set_file_name(file_name);
    tmp
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iso(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn empty_default_has_version_and_no_entries() {
        let store = SessionStore::default();
        assert_eq!(store.version, STORE_VERSION);
        assert!(store.entries.is_empty());
    }

    #[test]
    fn save_then_load_is_a_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        let mut original = SessionStore::default();
        original.entries.insert(
            "/Users/me/projects/blog".into(),
            SessionEntry {
                session_id: "abc-123".into(),
                claude_version: "1.0.0".into(),
                last_used: iso("2026-06-01T12:00:00Z"),
            },
        );
        original.save_atomic(&path).unwrap();

        let loaded = SessionStore::load_or_default(&path);
        assert_eq!(loaded, original);
    }

    #[test]
    fn load_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        let loaded = SessionStore::load_or_default(&path);
        assert_eq!(loaded, SessionStore::default());
    }

    #[test]
    fn load_garbage_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        fs::write(&path, "this is not json").unwrap();
        let loaded = SessionStore::load_or_default(&path);
        assert_eq!(loaded, SessionStore::default());
    }

    #[test]
    fn load_wrong_version_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        fs::write(&path, r#"{"version": 999, "entries": {}}"#).unwrap();
        let loaded = SessionStore::load_or_default(&path);
        assert_eq!(loaded, SessionStore::default());
    }

    #[test]
    fn upsert_inserts_and_updates() {
        let mut store = SessionStore::default();
        let project = Path::new("/tmp/proj");

        assert!(store.get(project).is_none());
        let inserted = store
            .upsert(project, "sid-1".into(), "1.0.0".into())
            .clone();
        assert_eq!(inserted.session_id, "sid-1");
        assert_eq!(inserted.claude_version, "1.0.0");

        // Second upsert replaces session_id; last_used should advance.
        let earlier = inserted.last_used;
        std::thread::sleep(std::time::Duration::from_millis(2));
        let updated = store
            .upsert(project, "sid-2".into(), "1.0.1".into())
            .clone();
        assert_eq!(updated.session_id, "sid-2");
        assert!(updated.last_used >= earlier);
        assert_eq!(store.entries.len(), 1, "same project must not duplicate");
    }

    #[test]
    fn save_creates_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/deep/sessions.json");
        let store = SessionStore::default();
        store.save_atomic(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn save_does_not_leave_tmp_file_on_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        SessionStore::default().save_atomic(&path).unwrap();
        let tmp = temp_path_for(&path);
        assert!(!tmp.exists(), "tmp file should be renamed away");
    }

    #[test]
    fn set_preview_round_trips_through_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        let mut store = SessionStore::default();
        store.set_preview("abc-123", Some(Path::new("/proj/docs/intro.md")));
        store.save_atomic(&path).unwrap();
        let loaded = SessionStore::load_or_default(&path);
        assert_eq!(
            loaded.get_preview("abc-123"),
            Some(Path::new("/proj/docs/intro.md")),
        );
        assert_eq!(loaded.get_preview("missing"), None);
    }

    #[test]
    fn set_preview_with_none_removes_the_entry() {
        let mut store = SessionStore::default();
        store.set_preview("sid", Some(Path::new("/a.md")));
        assert!(store.get_preview("sid").is_some());
        store.set_preview("sid", None);
        assert!(store.get_preview("sid").is_none());
    }

    #[test]
    fn load_old_sessions_json_without_previews_field() {
        // Sessions written before Phase 9.X have no session_previews
        // field. They must still deserialize cleanly so a user
        // upgrading mdpilot doesn't lose their saved entries.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.json");
        fs::write(
            &path,
            r#"{"version": 1, "entries": {"/proj": {"session_id": "sid-1", "claude_version": "1.0.0", "last_used": "2026-06-01T12:00:00Z"}}}"#,
        )
        .unwrap();
        let loaded = SessionStore::load_or_default(&path);
        assert_eq!(loaded.version, 1);
        assert!(loaded.entries.contains_key("/proj"));
        assert!(loaded.session_previews.is_empty());
    }
}
