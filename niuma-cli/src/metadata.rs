//! Projection of Codex desktop metadata into Niuma realtime sync messages.
//!
//! Codex remains the only history source. This module only reads Codex's saved
//! workspace roots and app-server thread lists, then emits thread-centric mobile
//! metadata.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::codex_app_server::{CodexAppServerClient, ModelState};
use crate::thread_status::normalize_thread_status;

const CONVERSATION_PROJECT_ID: &str = "__conversation__";

#[derive(Debug, Clone, Serialize)]
pub struct ProjectSummary {
    pub project_id: String,
    pub project_name: String,
    pub cwd: Option<String>,
    pub updated_at: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadSummary {
    pub thread_id: String,
    pub project_id: String,
    pub title: String,
    pub status: String,
    pub last_checkpoint_seen: Option<String>,
    pub updated_at: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct CodexThreadRecord {
    pub thread_id: String,
    pub project_id: String,
    pub cwd: Option<String>,
    pub title: String,
    pub status: String,
    pub last_checkpoint_seen: Option<String>,
    pub updated_at: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct MetadataSnapshot {
    pub projects: Vec<ProjectSummary>,
    pub threads: Vec<ThreadSummary>,
    pub model_state: ModelState,
}

#[derive(Debug, Deserialize)]
struct CodexGlobalState {
    #[serde(default, rename = "project-order")]
    project_order: Vec<String>,
    #[serde(default, rename = "electron-saved-workspace-roots")]
    electron_saved_workspace_roots: Vec<String>,
    #[serde(default, rename = "active-workspace-roots")]
    active_workspace_roots: Vec<String>,
    #[serde(default, rename = "electron-workspace-root-labels")]
    electron_workspace_root_labels: HashMap<String, String>,
    #[serde(default, rename = "projectless-thread-ids")]
    projectless_thread_ids: Vec<String>,
}

pub struct CodexWorkspaceStore {
    global_state_path: PathBuf,
}

impl CodexWorkspaceStore {
    /// Build a store using Codex desktop's persisted global state file.
    pub fn new() -> Self {
        Self {
            global_state_path: home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".codex")
                .join(".codex-global-state.json"),
        }
    }

    /// Return Codex workspace roots as Niuma project projections.
    pub fn list_projects(&self) -> Vec<ProjectSummary> {
        let state = self.read_state();
        let labels = normalized_labels(&state.electron_workspace_root_labels);
        ordered_roots(&state)
            .into_iter()
            .map(|root| ProjectSummary {
                project_id: project_id_for_root(&root),
                project_name: labels
                    .get(&root)
                    .cloned()
                    .unwrap_or_else(|| project_name_for_root(&root)),
                cwd: Some(root),
                updated_at: None,
            })
            .collect()
    }

    /// Find a workspace project by the mobile-facing project id.
    pub fn project_for_id(&self, project_id: &str) -> Option<ProjectSummary> {
        self.list_projects()
            .into_iter()
            .find(|project| project.project_id == project_id)
    }

    /// Find a workspace project by its Codex thread working directory.
    pub fn project_for_cwd(&self, cwd: &str) -> Option<ProjectSummary> {
        let normalized_cwd = normalize_root(cwd);
        self.list_projects()
            .into_iter()
            .find(|project| project.cwd.as_deref() == Some(normalized_cwd.as_str()))
    }

    /// Return Codex desktop's explicit conversation ids that have no project.
    pub fn list_projectless_thread_ids(&self) -> Vec<String> {
        self.read_state()
            .projectless_thread_ids
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect()
    }

    fn read_state(&self) -> CodexGlobalState {
        let Ok(payload) = std::fs::read_to_string(&self.global_state_path) else {
            return empty_state();
        };
        serde_json::from_str(&payload).unwrap_or_else(|_| empty_state())
    }
}

pub struct CodexMetadataProjector {
    app_server: CodexAppServerClient,
    workspace_store: CodexWorkspaceStore,
}

impl CodexMetadataProjector {
    /// Create the metadata projector from a live Codex app-server client.
    pub fn new(app_server: CodexAppServerClient) -> Self {
        Self {
            app_server,
            workspace_store: CodexWorkspaceStore::new(),
        }
    }

    /// Build one full list-screen snapshot from Codex workspaces and threads.
    pub async fn snapshot(&self) -> Result<MetadataSnapshot> {
        let mut projects = self.workspace_store.list_projects();
        let threads = self.list_all_threads(&projects).await?;
        let mut project_indices = HashMap::new();
        for (index, project) in projects.iter().enumerate() {
            project_indices.insert(project.project_id.clone(), index);
        }

        let mut thread_summaries = Vec::with_capacity(threads.len());
        for thread in threads {
            if let Some(index) = project_indices.get(&thread.project_id).copied() {
                let project_updated_at = projects[index].updated_at.unwrap_or(0.0);
                if thread.updated_at.unwrap_or(0.0) > project_updated_at {
                    projects[index].updated_at = thread.updated_at;
                }
            }
            thread_summaries.push(ThreadSummary {
                thread_id: thread.thread_id.clone(),
                project_id: thread.project_id.clone(),
                title: thread.title.clone(),
                status: thread.status.clone(),
                last_checkpoint_seen: thread.last_checkpoint_seen.clone(),
                updated_at: thread.updated_at,
            });
        }

        Ok(MetadataSnapshot {
            projects,
            threads: thread_summaries,
            model_state: self.app_server.model_state().await,
        })
    }

    async fn list_all_threads(
        &self,
        projects: &[ProjectSummary],
    ) -> Result<Vec<CodexThreadRecord>> {
        let mut threads_by_id = HashMap::new();
        for project in projects {
            for thread in self.list_threads_for_project(project).await? {
                threads_by_id.insert(thread.thread_id.clone(), thread);
            }
        }
        for thread in self.list_projectless_threads().await? {
            threads_by_id.insert(thread.thread_id.clone(), thread);
        }
        Ok(threads_by_id.into_values().collect())
    }

    async fn list_threads_for_project(
        &self,
        project: &ProjectSummary,
    ) -> Result<Vec<CodexThreadRecord>> {
        let Some(cwd) = project.cwd.as_deref() else {
            return Ok(Vec::new());
        };
        let mut threads_by_id = HashMap::new();
        for archived in [false, true] {
            for payload in self.app_server.list_thread_payloads(cwd, archived).await? {
                let thread = project_thread(
                    &payload,
                    Some(&project.project_id),
                    Some(cwd),
                    Some(archived),
                )?;
                threads_by_id.insert(thread.thread_id.clone(), thread);
            }
        }
        Ok(threads_by_id.into_values().collect())
    }

    async fn list_projectless_threads(&self) -> Result<Vec<CodexThreadRecord>> {
        let mut threads = Vec::new();
        for thread_id in self.workspace_store.list_projectless_thread_ids() {
            let raw = self
                .app_server
                .read_thread_payload(&thread_id, false)
                .await
                .with_context(|| format!("failed to read projectless thread {thread_id}"))?;
            let thread = project_thread(&raw, Some(CONVERSATION_PROJECT_ID), None, None)?;
            match self.find_thread_in_cwd(&thread).await? {
                Some(scoped) => threads.push(scoped),
                None => threads.push(thread),
            }
        }
        Ok(threads)
    }

    async fn find_thread_in_cwd(
        &self,
        target: &CodexThreadRecord,
    ) -> Result<Option<CodexThreadRecord>> {
        let Some(cwd) = target.cwd.as_deref() else {
            return Ok(None);
        };
        let project_id = self
            .workspace_store
            .project_for_cwd(cwd)
            .map(|project| project.project_id)
            .unwrap_or_else(|| CONVERSATION_PROJECT_ID.to_string());
        for archived in [false, true] {
            for payload in self.app_server.list_thread_payloads(cwd, archived).await? {
                let thread =
                    project_thread(&payload, Some(&project_id), Some(cwd), Some(archived))?;
                if thread.thread_id == target.thread_id {
                    return Ok(Some(thread));
                }
            }
        }
        Ok(None)
    }
}

/// Convert a snapshot into the server's existing websocket event objects.
pub fn snapshot_wire_messages(snapshot: &MetadataSnapshot) -> Vec<Value> {
    let mut messages = Vec::new();
    for project in &snapshot.projects {
        messages.push(json!({
            "kind": "project_sync",
            "project_id": project.project_id,
            "project_name": project.project_name,
            "updated_at": project.updated_at,
        }));
    }
    for thread in &snapshot.threads {
        let record = CodexThreadRecord {
            thread_id: thread.thread_id.clone(),
            project_id: thread.project_id.clone(),
            cwd: None,
            title: thread.title.clone(),
            status: thread.status.clone(),
            last_checkpoint_seen: thread.last_checkpoint_seen.clone(),
            updated_at: thread.updated_at,
        };
        messages.extend(thread_metadata_wire_messages(&record));
    }
    messages.push(json!({
        "kind": "model_sync",
        "current_model": snapshot.model_state.current_model,
        "available_models": snapshot.model_state.available_models,
    }));
    messages
}

/// Convert one projected Codex thread into the single mobile sync message.
pub fn thread_metadata_wire_messages(thread: &CodexThreadRecord) -> Vec<Value> {
    vec![json!({
        "kind": "thread_sync",
        "thread_id": thread.thread_id,
        "project_id": thread.project_id,
        "title": thread.title,
        "status": thread.status,
        "last_checkpoint_seen": thread.last_checkpoint_seen,
        "updated_at": thread.updated_at,
    })]
}

fn project_thread(
    payload: &Value,
    project_id: Option<&str>,
    cwd: Option<&str>,
    archived_override: Option<bool>,
) -> Result<CodexThreadRecord> {
    let thread_id = string_field(payload, "id").context("thread payload missing id")?;
    let resolved_cwd = cwd
        .map(str::to_string)
        .or_else(|| string_field(payload, "cwd"));
    let resolved_project_id = project_id.unwrap_or(&thread_id).to_string();
    let title = string_field(payload, "name")
        .or_else(|| string_field(payload, "preview"))
        .unwrap_or_else(|| thread_id.clone());
    let archived = archived_override.unwrap_or_else(|| bool_field(payload, "archived"));
    Ok(CodexThreadRecord {
        thread_id,
        project_id: resolved_project_id,
        cwd: resolved_cwd,
        title,
        status: normalize_thread_status(payload.get("status"), archived),
        last_checkpoint_seen: None,
        updated_at: number_field(payload, "updatedAt").or_else(|| Some(unix_timestamp())),
    })
}

fn empty_state() -> CodexGlobalState {
    CodexGlobalState {
        project_order: Vec::new(),
        electron_saved_workspace_roots: Vec::new(),
        active_workspace_roots: Vec::new(),
        electron_workspace_root_labels: HashMap::new(),
        projectless_thread_ids: Vec::new(),
    }
}

fn ordered_roots(state: &CodexGlobalState) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut roots = Vec::new();
    for root in state
        .project_order
        .iter()
        .chain(state.electron_saved_workspace_roots.iter())
        .chain(state.active_workspace_roots.iter())
    {
        let normalized = normalize_root(root);
        if !normalized.is_empty() && seen.insert(normalized.clone()) {
            roots.push(normalized);
        }
    }
    roots
}

fn normalized_labels(labels: &HashMap<String, String>) -> HashMap<String, String> {
    labels
        .iter()
        .filter(|(key, value)| !key.is_empty() && !value.is_empty())
        .map(|(key, value)| (normalize_root(key), value.clone()))
        .collect()
}

fn normalize_root(root: &str) -> String {
    let expanded = expand_home(root);
    let path = PathBuf::from(expanded);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    absolute.to_string_lossy().into_owned()
}

fn expand_home(root: &str) -> String {
    if root == "~" {
        return home_dir()
            .unwrap_or_else(|| PathBuf::from("~"))
            .to_string_lossy()
            .into_owned();
    }
    if let Some(rest) = root.strip_prefix("~/") {
        return home_dir()
            .unwrap_or_else(|| PathBuf::from("~"))
            .join(rest)
            .to_string_lossy()
            .into_owned();
    }
    root.to_string()
}

fn project_name_for_root(root: &str) -> String {
    Path::new(root)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(root)
        .to_string()
}

fn project_id_for_root(root: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(root.as_bytes());
    let digest = hasher.finalize();
    format!("workspace-{}", hex::encode(digest)[..16].to_string())
}

fn string_field(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn bool_field(payload: &Value, key: &str) -> bool {
    payload.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn number_field(payload: &Value, key: &str) -> Option<f64> {
    payload
        .get(key)
        .and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()))
}

fn unix_timestamp() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_store_orders_and_deduplicates_roots() {
        let root = std::env::temp_dir().join(format!("niuma-metadata-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let state_path = root.join("state.json");
        std::fs::write(
            &state_path,
            serde_json::to_string(&json!({
                "project-order": ["/tmp/a", "/tmp/b"],
                "electron-saved-workspace-roots": ["/tmp/b", "/tmp/c"],
                "active-workspace-roots": ["/tmp/a"],
                "electron-workspace-root-labels": {"/tmp/b": "Bee"},
                "projectless-thread-ids": ["thread-1", ""],
            }))
            .unwrap(),
        )
        .unwrap();

        let store = CodexWorkspaceStore {
            global_state_path: state_path,
        };
        let projects = store.list_projects();
        assert_eq!(
            projects
                .iter()
                .map(|project| project.cwd.as_deref().unwrap())
                .collect::<Vec<_>>(),
            vec!["/tmp/a", "/tmp/b", "/tmp/c"]
        );
        assert_eq!(projects[1].project_name, "Bee");
        assert_eq!(store.list_projectless_thread_ids(), vec!["thread-1"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn archived_thread_emits_archived_thread_signal() {
        let messages = thread_metadata_wire_messages(&CodexThreadRecord {
            thread_id: "thread-1".to_string(),
            project_id: "project-1".to_string(),
            cwd: Some("/tmp/project".to_string()),
            title: "Archived".to_string(),
            status: "archived".to_string(),
            last_checkpoint_seen: None,
            updated_at: Some(1.0),
        });

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["kind"], "thread_sync");
        assert_eq!(messages[0]["thread_id"], "thread-1");
        assert_eq!(messages[0]["status"], "archived");
    }
}
