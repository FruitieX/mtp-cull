use crate::review::{Decision, Settings};
use color_eyre::eyre::{Result, WrapErr, eyre};
use directories::ProjectDirs;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedReview {
    pub decisions: HashMap<String, Decision>,
    pub selected: Option<String>,
    pub pinned: Option<String>,
    pub center: Option<[f32; 2]>,
    pub zoom: Option<f32>,
    pub alignment: [f32; 2],
    pub mode: String,
    pub threshold: u8,
    pub opacity: u8,
}

enum Write {
    Settings(Settings),
    Session(String, SavedReview),
    Shutdown,
}
pub struct Store {
    path: PathBuf,
    sender: Sender<Write>,
    errors: Receiver<String>,
    thread: Option<JoinHandle<()>>,
    pending: std::cell::RefCell<HashMap<String, SavedReview>>,
}
fn connect(path: &std::path::Path) -> Result<Connection> {
    let connection = Connection::open(path)?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    connection.execute_batch("PRAGMA journal_mode=WAL;
        CREATE TABLE IF NOT EXISTS review_settings (id INTEGER PRIMARY KEY, json TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS review_sessions (id TEXT PRIMARY KEY, json TEXT NOT NULL, updated INTEGER NOT NULL);")?;
    Ok(connection)
}
impl Store {
    pub fn open() -> Result<Self> {
        if let Some(root) = std::env::var_os("MTP_CULL_DATA_DIR") {
            let root = PathBuf::from(root);
            std::fs::create_dir_all(&root)?;
            return Self::at(root.join("cull.sqlite3"));
        }
        let dirs = ProjectDirs::from("com", "fruit", "mtp-cull")
            .ok_or_else(|| eyre!("cannot find application directory"))?;
        std::fs::create_dir_all(dirs.data_local_dir())?;
        Self::at(dirs.data_local_dir().join("cull.sqlite3"))
    }
    pub(crate) fn at(path: PathBuf) -> Result<Self> {
        let connection = connect(&path).wrap_err("could not open review database")?;
        let (sender, receiver) = mpsc::channel();
        let (errors_sender, errors) = mpsc::channel();
        let thread = std::thread::Builder::new().name("review-persistence".into()).spawn(move || {
            while let Ok(write) = receiver.recv() {
                let result: Result<()> = (|| {
                    match write {
                        Write::Settings(settings) => {
                            connection.execute("INSERT INTO review_settings(id,json) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json", [serde_json::to_string(&settings)?])?;
                        }
                        Write::Session(id, decisions) => {
                            connection.execute("INSERT INTO review_sessions(id,json,updated) VALUES(?1,?2,unixepoch()) ON CONFLICT(id) DO UPDATE SET json=excluded.json,updated=excluded.updated", params![id, serde_json::to_string(&decisions)?])?;
                        }
                        Write::Shutdown => {}
                    }
                    Ok(())
                })();
                if let Err(error) = result { let _ = errors_sender.send(format!("Could not save review: {error:#}")); }
            }
        })?;
        Ok(Self {
            path,
            sender,
            errors,
            thread: Some(thread),
            pending: std::cell::RefCell::new(HashMap::new()),
        })
    }
    pub fn settings(&self) -> Result<Settings> {
        let json: Option<String> = connect(&self.path)?
            .query_row("SELECT json FROM review_settings WHERE id=1", [], |r| {
                r.get(0)
            })
            .optional()?;
        let legacy_defaults = json.as_ref().is_some_and(|j| {
            serde_json::from_str::<serde_json::Value>(j)
                .ok()
                .is_some_and(|v| {
                    v.get("shortcut_defaults_version")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                        < 1
                })
        });
        let mut settings: Settings = json
            .map(|j| serde_json::from_str(&j))
            .transpose()?
            .unwrap_or_default();
        // Earlier editors populated both all-filter commands under the same ID.
        // Shift+A was inserted first, unintentionally overriding the media A binding.
        if !settings.bindings.contains_key("decision_filter_all")
            && settings
                .bindings
                .get("filter_all")
                .is_some_and(|key| key == "Shift+A")
        {
            let key = settings.bindings.remove("filter_all").unwrap();
            settings.bindings.insert("decision_filter_all".into(), key);
        }
        if legacy_defaults {
            // Migrate only the old default pair; retain deliberately customized keys.
            let keep = settings.bindings.get("keep").map_or("2", String::as_str);
            let reject = settings.bindings.get("reject").map_or("1", String::as_str);
            if keep == "2" && reject == "1" {
                settings.bindings.remove("keep");
                settings.bindings.remove("reject");
            }
            if settings
                .bindings
                .get("bulk_keep")
                .is_some_and(|k| k == "Ctrl+A")
            {
                settings.bindings.remove("bulk_keep");
            }
            settings.shortcut_defaults_version = 1;
        }
        Ok(settings)
    }
    pub fn review(&self, id: &str) -> Result<SavedReview> {
        if let Some(review) = self.pending.borrow().get(id) {
            return Ok(review.clone());
        }
        let json: Option<String> = connect(&self.path)?
            .query_row("SELECT json FROM review_sessions WHERE id=?1", [id], |r| {
                r.get(0)
            })
            .optional()?;
        json.map(|json| {
            let value: serde_json::Value = serde_json::from_str(&json)?;
            if value.get("decisions").is_some() {
                Ok(serde_json::from_value(value)?)
            } else {
                Ok(SavedReview {
                    decisions: serde_json::from_value(value)?,
                    ..SavedReview::default()
                })
            }
        })
        .unwrap_or_else(|| Ok(SavedReview::default()))
    }
    pub fn save_settings(&self, settings: &Settings) {
        let _ = self.sender.send(Write::Settings(settings.clone()));
    }
    pub fn save_review(&self, id: &str, review: SavedReview) {
        self.pending
            .borrow_mut()
            .insert(id.to_owned(), review.clone());
        let _ = self.sender.send(Write::Session(id.to_owned(), review));
    }
    pub fn error(&self) -> Option<String> {
        self.errors.try_recv().ok()
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        // Disconnect after all preceding writes; the receiver drains FIFO before exit.
        let (replacement, _) = mpsc::channel();
        let previous = std::mem::replace(&mut self.sender, replacement);
        let _ = previous.send(Write::Shutdown);
        drop(previous);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_default_decision_and_bulk_keys_migrate_but_custom_keys_survive() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("review.sqlite3");
        let connection = connect(&path).unwrap();
        for (bindings, expected_keep, expected_reject) in [
            (
                r#"{"keep":"2","reject":"1","bulk_keep":"Ctrl+A"}"#,
                "1",
                "2",
            ),
            (
                r#"{"keep":"3","reject":"4","bulk_keep":"Ctrl+Shift+L"}"#,
                "3",
                "4",
            ),
            (r#"{"keep":"1","reject":"2"}"#, "1", "2"),
        ] {
            let json = format!("{{\"bindings\":{bindings}}}");
            connection
                .execute(
                    "INSERT OR REPLACE INTO review_settings(id,json) VALUES(1,?1)",
                    [json],
                )
                .unwrap();
            let store = Store::at(path.clone()).unwrap();
            let settings = store.settings().unwrap();
            let key = |id: &str| {
                settings
                    .bindings
                    .get(id)
                    .map(String::as_str)
                    .unwrap_or_else(|| {
                        crate::review_commands::COMMANDS
                            .iter()
                            .find(|s| s.id == id)
                            .unwrap()
                            .key
                    })
            };
            assert_eq!(key("keep"), expected_keep);
            assert_eq!(key("reject"), expected_reject);
            assert_eq!(key("select_all"), "Ctrl+A");
            assert_ne!(key("bulk_keep"), "Ctrl+A");
            assert_eq!(settings.shortcut_defaults_version, 1);
            crate::review_commands::validate(&settings.bindings).unwrap();
            // Saving the new defaults allows future deliberate reverse bindings.
            store.save_settings(&settings);
        }
    }
    #[test]
    fn recent_sources_survive_restart_and_older_settings_default_to_empty_history() {
        use crate::recent_sources::RecentSource;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("review.sqlite3");
        let sources = vec![
            RecentSource::Local {
                root: "C:/Photos/session".into(),
                raw: Some("C:/Photos/RAW".into()),
            },
            RecentSource::Camera {
                device_id: "device-id".into(),
                device_name: "Fujifilm X-T5".into(),
                folder_id: "folder-id".into(),
                folder_path: "DCIM/100_FUJI".into(),
            },
        ];
        {
            let store = Store::at(path.clone()).unwrap();
            store.save_settings(&Settings {
                colourblind: true,
                reel_position: crate::reel::Position::Right,
                reel_width: 420.0,
                recent_sources: sources.clone(),
                ..Settings::default()
            });
        }
        {
            let store = Store::at(path.clone()).unwrap();
            assert_eq!(store.settings().unwrap().recent_sources, sources);
            assert!(store.settings().unwrap().colourblind);
            assert_eq!(
                store.settings().unwrap().reel_position,
                crate::reel::Position::Right
            );
            assert_eq!(store.settings().unwrap().reel_width, 420.0);
        }
        // A pre-history settings record still loads its other preferences.
        connect(&path)
            .unwrap()
            .execute(
                "UPDATE review_settings SET json=?1 WHERE id=1",
                [r#"{"auto_advance":true}"#],
            )
            .unwrap();
        let store = Store::at(path).unwrap();
        let settings = store.settings().unwrap();
        assert!(settings.recent_sources.is_empty());
        assert!(!settings.colourblind);
        assert_eq!(settings.reel_position, crate::reel::Position::Bottom);
        assert_eq!(settings.reel_width, Settings::default().reel_width);
        assert!(settings.auto_advance);
    }
    #[test]
    fn legacy_all_filter_binding_is_split_without_losing_customizations() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("review.sqlite3");
        {
            let store = Store::at(path.clone()).unwrap();
            let mut settings = Settings::default();
            settings
                .bindings
                .insert("filter_all".into(), "Shift+A".into());
            settings.bindings.insert("keep".into(), "3".into());
            store.save_settings(&settings);
        }
        let store = Store::at(path).unwrap();
        let settings = store.settings().unwrap();
        assert!(!settings.bindings.contains_key("filter_all"));
        assert_eq!(settings.bindings["decision_filter_all"], "Shift+A");
        assert_eq!(settings.bindings["keep"], "3");
        crate::review_commands::validate(&settings.bindings).unwrap();
    }
    #[test]
    fn writes_flush_on_close_and_other_sessions_are_isolated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("review.sqlite3");
        {
            let store = Store::at(path.clone()).unwrap();
            store.save_review(
                "one",
                SavedReview {
                    decisions: HashMap::from([("asset".into(), Decision::Keep)]),
                    selected: Some("asset".into()),
                    ..SavedReview::default()
                },
            );
            assert_eq!(
                store.review("one").unwrap().selected.as_deref(),
                Some("asset")
            );
            store.save_settings(&Settings {
                auto_advance: true,
                ..Settings::default()
            });
        }
        let store = Store::at(path).unwrap();
        assert_eq!(
            store.review("one").unwrap().decisions["asset"],
            Decision::Keep
        );
        assert_eq!(
            store.review("one").unwrap().selected.as_deref(),
            Some("asset")
        );
        assert!(store.review("two").unwrap().decisions.is_empty());
        assert!(store.settings().unwrap().auto_advance);
    }
}
