//! On-disk persistence. Layout (under %APPDATA%\Tessera\data):
//!   workspace.json         environments, globals, active env, collection order
//!   collections/<id>.json  one file per collection
//!   history.json           request history
//!   settings.json          settings + client certificates
//!   ui.json                UI session state (open tabs etc.)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::model::{ClientCert, Collection, Environment, HistoryEntry, KeyValue, Settings, Workspace};

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct WorkspaceFile {
    version: u32,
    collection_order: Vec<String>,
    environments: Vec<Environment>,
    globals: Vec<KeyValue>,
    active_env: Option<String>,
    flows: Vec<crate::flow::Flow>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct SettingsFile {
    settings: Settings,
    certificates: Vec<ClientCert>,
}

pub struct Storage {
    root: PathBuf,
    /// Last content written per file, so unchanged files aren't rewritten.
    written: Mutex<HashMap<PathBuf, String>>,
}

impl Storage {
    /// `TESSERA_DATA_DIR` overrides the location (portable installs, testing).
    pub fn default_root() -> PathBuf {
        if let Some(dir) = std::env::var_os("TESSERA_DATA_DIR").filter(|d| !d.is_empty()) {
            return PathBuf::from(dir);
        }
        directories::ProjectDirs::from("dev", "Tessera", "Tessera")
            .map(|d| d.data_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("tessera-data"))
    }

    /// The app used to be called Swiftpost: move its data folder over once, if present.
    pub fn migrate_legacy_data(root: &Path) {
        if root.exists() || std::env::var_os("TESSERA_DATA_DIR").is_some() {
            return;
        }
        let Some(old) = directories::ProjectDirs::from("dev", "Swiftpost", "Swiftpost").map(|d| d.data_dir().to_path_buf()) else {
            return;
        };
        if old.exists() {
            if let Some(parent) = root.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::rename(&old, root) {
                eprintln!("tessera: could not migrate {} -> {}: {e}", old.display(), root.display());
            }
        }
    }

    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), written: Mutex::new(HashMap::new()) }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn read<T: DeserializeOwned + Default>(&self, path: &Path) -> T {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                self.written.lock().unwrap().insert(path.to_path_buf(), text.clone());
                serde_json::from_str(&text).unwrap_or_else(|e| {
                    // Keep the unreadable file around rather than overwriting it silently.
                    let backup = path.with_extension(format!("broken-{}.json", chrono::Local::now().format("%Y%m%d%H%M%S")));
                    let _ = std::fs::copy(path, backup);
                    eprintln!("tessera: could not parse {}: {e}", path.display());
                    T::default()
                })
            }
            Err(_) => T::default(),
        }
    }

    fn write(&self, path: &Path, text: String) -> std::io::Result<()> {
        if self.written.lock().unwrap().get(path) == Some(&text) {
            return Ok(());
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &text)?;
        std::fs::rename(&tmp, path)?;
        self.written.lock().unwrap().insert(path.to_path_buf(), text);
        Ok(())
    }

    fn write_json<T: Serialize>(&self, path: &Path, value: &T) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
        self.write(path, text)
    }

    pub fn load(&self) -> Workspace {
        let wf: WorkspaceFile = self.read(&self.root.join("workspace.json"));
        let sf: SettingsFile = self.read(&self.root.join("settings.json"));
        let history: Vec<HistoryEntry> = self.read(&self.root.join("history.json"));
        let mut collections: Vec<Collection> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(self.root.join("collections")) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "json") {
                    let c: Option<Collection> = self.read(&p);
                    if let Some(c) = c {
                        collections.push(c);
                    }
                }
            }
        }
        collections.sort_by_key(|c| {
            wf.collection_order.iter().position(|id| id == &c.id).unwrap_or(usize::MAX)
        });
        Workspace {
            collections,
            environments: wf.environments,
            globals: wf.globals,
            active_env: wf.active_env,
            flows: wf.flows,
            history,
            settings: sf.settings,
            certificates: sf.certificates,
        }
    }

    pub fn save(&self, ws: &Workspace) -> std::io::Result<()> {
        let wf = WorkspaceFile {
            version: 1,
            collection_order: ws.collections.iter().map(|c| c.id.clone()).collect(),
            environments: ws.environments.clone(),
            globals: ws.globals.clone(),
            active_env: ws.active_env.clone(),
            flows: ws.flows.clone(),
        };
        self.write_json(&self.root.join("workspace.json"), &wf)?;
        self.write_json(
            &self.root.join("settings.json"),
            &SettingsFile { settings: ws.settings.clone(), certificates: ws.certificates.clone() },
        )?;
        let history = serde_json::to_string(&ws.history).map_err(std::io::Error::other)?;
        self.write(&self.root.join("history.json"), history)?;

        let dir = self.root.join("collections");
        for c in &ws.collections {
            self.write_json(&dir.join(format!("{}.json", c.id)), c)?;
        }
        // Remove files of deleted collections.
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if p.extension().is_some_and(|x| x == "json") && !ws.collections.iter().any(|c| c.id == stem) {
                    let _ = std::fs::remove_file(&p);
                    self.written.lock().unwrap().remove(&p);
                }
            }
        }
        Ok(())
    }

    pub fn load_ui<T: DeserializeOwned + Default>(&self) -> T {
        self.read(&self.root.join("ui.json"))
    }

    pub fn save_ui<T: Serialize>(&self, value: &T) -> std::io::Result<()> {
        self.write_json(&self.root.join("ui.json"), value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Item, Request};

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("tessera-test-{}", crate::model::new_id()));
        let st = Storage::new(&dir);
        let mut ws = Workspace::default();
        let mut a = Collection::new("A");
        a.items.push(Item::Request(Request::default()));
        let b = Collection::new("B");
        ws.collections = vec![b, a];
        ws.environments.push(Environment::new("dev"));
        ws.push_history(Request::default(), 200, 12.0);
        st.save(&ws).unwrap();
        let loaded = Storage::new(&dir).load();
        assert_eq!(loaded, ws);
        // Deleting a collection removes its file.
        ws.collections.remove(0);
        st.save(&ws).unwrap();
        assert_eq!(Storage::new(&dir).load().collections.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
