//! Unified diff parsing and mobile summary projection.
//!
//! Codex app-server exposes file changes as raw unified diff strings. The
//! mobile UI needs compact counts for timeline cards and a structured bundle
//! for file detail sheets, so parsing is centralized here instead of split
//! across realtime handlers.

use anyhow::{Context, Result};
use regex::Regex;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use tokio::process::Command;

const MAX_UNTRACKED_RAW_DIFF_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone)]
pub struct BranchChanges {
    pub summary: Value,
    pub files_summary: Value,
    pub bundle: Value,
}

#[derive(Debug, Clone)]
struct ParsedFileDiff {
    path: String,
    old_path: Option<String>,
    change_type: String,
    additions: i64,
    deletions: i64,
    raw_diff: String,
    hunks: Vec<Value>,
}

/// Build the `file_change_summary` content part for the final answer's
/// associated Codex app-server `fileChange` group.
pub fn file_change_summary_part(
    turn_id: &str,
    final_answer_entry_id: &str,
    file_change_items: &[Value],
    cwd: Option<&str>,
) -> Option<Value> {
    let mut files = BTreeMap::<String, ParsedFileDiff>::new();
    for item in file_change_items {
        for change in item
            .get("changes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let parsed = parsed_app_server_change(change, cwd)?;
            merge_file_diff(&mut files, parsed);
        }
    }
    if files.is_empty() {
        return None;
    }

    let summary = summary_value(&files);
    let files_summary = files_summary_value(&files);
    let bundle = json!({
        "version": 1,
        "source": "codex_app_server_file_change",
        "turn_id": turn_id,
        "final_answer_entry_id": final_answer_entry_id,
        "summary": summary,
        "files": files_bundle_value(&files),
    });
    Some(json!({
        "type": "file_change_summary",
        "files": summary.get("files").cloned().unwrap_or_else(|| json!(0)),
        "additions": summary.get("additions").cloned().unwrap_or_else(|| json!(0)),
        "deletions": summary.get("deletions").cloned().unwrap_or_else(|| json!(0)),
        "files_summary": files_summary,
        "diff_bundle": bundle,
    }))
}

/// Compute the current project branch/worktree changes directly from Git.
pub async fn branch_changes(cwd: &str, base_ref: Option<&str>) -> Result<BranchChanges> {
    let diff = git_diff(cwd, base_ref).await?;
    let mut files = parse_git_diff(&diff);
    for parsed in untracked_file_diffs(cwd).await? {
        merge_file_diff(&mut files, parsed);
    }
    let summary = summary_value(&files);
    let files_summary = files_summary_value(&files);
    let bundle = json!({
        "version": 1,
        "source": "git_branch_changes",
        "base_ref": base_ref,
        "summary": summary,
        "files": files_bundle_value(&files),
    });
    Ok(BranchChanges {
        summary,
        files_summary,
        bundle,
    })
}

fn parsed_app_server_change(change: &Value, cwd: Option<&str>) -> Option<ParsedFileDiff> {
    let path = change
        .get("path")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())?;
    let raw_diff = change
        .get("diff")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let (additions, deletions, hunks) = parse_unified_diff(&raw_diff);
    Some(ParsedFileDiff {
        path: display_path(path, cwd),
        old_path: change
            .get("kind")
            .and_then(|kind| kind.get("move_path"))
            .and_then(Value::as_str)
            .map(|value| display_path(value, cwd)),
        change_type: change_type(change),
        additions,
        deletions,
        raw_diff,
        hunks,
    })
}

fn change_type(change: &Value) -> String {
    change
        .get("kind")
        .and_then(|kind| {
            kind.get("type")
                .and_then(Value::as_str)
                .or_else(|| kind.as_str())
        })
        .unwrap_or("update")
        .to_string()
}

fn merge_file_diff(files: &mut BTreeMap<String, ParsedFileDiff>, parsed: ParsedFileDiff) {
    match files.get_mut(&parsed.path) {
        Some(existing) => {
            existing.additions += parsed.additions;
            existing.deletions += parsed.deletions;
            if !parsed.raw_diff.is_empty() {
                if !existing.raw_diff.is_empty() {
                    existing.raw_diff.push('\n');
                }
                existing.raw_diff.push_str(&parsed.raw_diff);
            }
            existing.hunks.extend(parsed.hunks);
            existing.change_type = parsed.change_type;
            if existing.old_path.is_none() {
                existing.old_path = parsed.old_path;
            }
        }
        None => {
            files.insert(parsed.path.clone(), parsed);
        }
    }
}

fn parse_unified_diff(diff: &str) -> (i64, i64, Vec<Value>) {
    let mut additions = 0;
    let mut deletions = 0;
    let mut hunks = Vec::new();
    let mut current: Option<HunkBuilder> = None;

    for line in diff.lines() {
        if let Some(header) = parse_hunk_header(line) {
            if let Some(hunk) = current.take().and_then(HunkBuilder::finish) {
                hunks.push(hunk);
            }
            current = Some(HunkBuilder::new(header));
            continue;
        }
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            additions += 1;
            if let Some(hunk) = current.as_mut() {
                hunk.push_add(line);
            }
            continue;
        }
        if line.starts_with('-') {
            deletions += 1;
            if let Some(hunk) = current.as_mut() {
                hunk.push_delete(line);
            }
            continue;
        }
        if line.starts_with(' ') {
            if let Some(hunk) = current.as_mut() {
                hunk.push_context(line);
            }
        }
    }
    if let Some(hunk) = current.take().and_then(HunkBuilder::finish) {
        hunks.push(hunk);
    }
    (additions, deletions, hunks)
}

#[derive(Debug, Clone, Copy)]
struct HunkHeader {
    old_start: i64,
    old_lines: i64,
    new_start: i64,
    new_lines: i64,
}

fn parse_hunk_header(line: &str) -> Option<HunkHeader> {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let captures = PATTERN
        .get_or_init(|| {
            Regex::new(r"^@@ -(?P<old>\d+)(?:,(?P<oldn>\d+))? \+(?P<new>\d+)(?:,(?P<newn>\d+))? @@")
                .expect("valid unified diff hunk header pattern")
        })
        .captures(line)?;
    Some(HunkHeader {
        old_start: captures.name("old")?.as_str().parse().ok()?,
        old_lines: captures
            .name("oldn")
            .map(|value| value.as_str().parse().unwrap_or(1))
            .unwrap_or(1),
        new_start: captures.name("new")?.as_str().parse().ok()?,
        new_lines: captures
            .name("newn")
            .map(|value| value.as_str().parse().unwrap_or(1))
            .unwrap_or(1),
    })
}

struct HunkBuilder {
    header: HunkHeader,
    old_line: i64,
    new_line: i64,
    lines: Vec<Value>,
}

impl HunkBuilder {
    fn new(header: HunkHeader) -> Self {
        Self {
            old_line: header.old_start,
            new_line: header.new_start,
            header,
            lines: Vec::new(),
        }
    }

    fn push_context(&mut self, line: &str) {
        self.lines.push(json!({
            "kind": "context",
            "old_line": self.old_line,
            "new_line": self.new_line,
            "content": trim_diff_prefix(line),
        }));
        self.old_line += 1;
        self.new_line += 1;
    }

    fn push_delete(&mut self, line: &str) {
        self.lines.push(json!({
            "kind": "delete",
            "old_line": self.old_line,
            "new_line": null,
            "content": trim_diff_prefix(line),
        }));
        self.old_line += 1;
    }

    fn push_add(&mut self, line: &str) {
        self.lines.push(json!({
            "kind": "add",
            "old_line": null,
            "new_line": self.new_line,
            "content": trim_diff_prefix(line),
        }));
        self.new_line += 1;
    }

    fn finish(self) -> Option<Value> {
        Some(json!({
            "old_start": self.header.old_start,
            "old_lines": self.header.old_lines,
            "new_start": self.header.new_start,
            "new_lines": self.header.new_lines,
            "lines": self.lines,
        }))
    }
}

fn trim_diff_prefix(line: &str) -> &str {
    line.get(1..).unwrap_or("")
}

fn summary_value(files: &BTreeMap<String, ParsedFileDiff>) -> Value {
    json!({
        "files": files.len() as i64,
        "additions": files.values().map(|file| file.additions).sum::<i64>(),
        "deletions": files.values().map(|file| file.deletions).sum::<i64>(),
    })
}

fn files_summary_value(files: &BTreeMap<String, ParsedFileDiff>) -> Value {
    Value::Array(
        files
            .values()
            .map(|file| {
                json!({
                    "path": file.path,
                    "change_type": file.change_type,
                    "additions": file.additions,
                    "deletions": file.deletions,
                })
            })
            .collect(),
    )
}

fn files_bundle_value(files: &BTreeMap<String, ParsedFileDiff>) -> Value {
    Value::Array(
        files
            .values()
            .map(|file| {
                json!({
                    "path": file.path,
                    "old_path": file.old_path,
                    "change_type": file.change_type,
                    "additions": file.additions,
                    "deletions": file.deletions,
                    "raw_diff": file.raw_diff,
                    "hunks": file.hunks,
                })
            })
            .collect(),
    )
}

fn display_path(path: &str, cwd: Option<&str>) -> String {
    let trimmed = path.trim();
    if let Some(cwd) = cwd {
        if let Ok(relative) = Path::new(trimmed).strip_prefix(Path::new(cwd)) {
            return relative.to_string_lossy().to_string();
        }
    }
    trimmed.to_string()
}

async fn git_diff(cwd: &str, base_ref: Option<&str>) -> Result<String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(cwd)
        .arg("diff")
        .arg("--no-ext-diff")
        .arg("--find-renames");
    match base_ref.filter(|value| !value.trim().is_empty()) {
        Some(base_ref) => {
            command.arg(format!("{base_ref}...HEAD"));
        }
        None => {
            command.arg("HEAD");
        }
    }
    let output = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("failed to run git diff in {cwd}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "git diff failed in {cwd}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn git_untracked_paths(cwd: &str) -> Result<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .arg("ls-files")
        .arg("--others")
        .arg("--exclude-standard")
        .arg("-z")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("failed to list untracked files in {cwd}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "git ls-files failed in {cwd}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).to_string())
        .collect())
}

async fn untracked_file_diffs(cwd: &str) -> Result<Vec<ParsedFileDiff>> {
    let mut files = Vec::new();
    for path in git_untracked_paths(cwd).await? {
        let absolute_path = Path::new(cwd).join(&path);
        if !absolute_path.is_file() {
            continue;
        }
        files.push(untracked_file_diff(path, absolute_path)?);
    }
    Ok(files)
}

fn untracked_file_diff(path: String, absolute_path: PathBuf) -> Result<ParsedFileDiff> {
    let metadata = std::fs::metadata(&absolute_path)
        .with_context(|| format!("failed to read metadata for {}", absolute_path.display()))?;
    let additions = text_line_count(&absolute_path)?.unwrap_or(0);
    let (raw_diff, hunks) = if metadata.len() <= MAX_UNTRACKED_RAW_DIFF_BYTES {
        match std::fs::read(&absolute_path) {
            Ok(bytes) if !bytes.contains(&0) => match String::from_utf8(bytes) {
                Ok(text) => {
                    let raw_diff = untracked_text_raw_diff(&path, &text);
                    let (_, _, hunks) = parse_unified_diff(&raw_diff);
                    (raw_diff, hunks)
                }
                Err(_) => (String::new(), Vec::new()),
            },
            _ => (String::new(), Vec::new()),
        }
    } else {
        (String::new(), Vec::new())
    };
    Ok(ParsedFileDiff {
        path,
        old_path: None,
        change_type: "create".to_string(),
        additions,
        deletions: 0,
        raw_diff,
        hunks,
    })
}

fn text_line_count(path: &Path) -> Result<Option<i64>> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("failed to open untracked file {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut buffer = Vec::new();
    let mut lines = 0;
    loop {
        buffer.clear();
        let read = reader
            .read_until(b'\n', &mut buffer)
            .with_context(|| format!("failed to read untracked file {}", path.display()))?;
        if read == 0 {
            break;
        }
        if buffer.contains(&0) {
            return Ok(None);
        }
        lines += 1;
    }
    Ok(Some(lines))
}

fn untracked_text_raw_diff(path: &str, text: &str) -> String {
    let line_count = text.lines().count();
    let mut diff = format!(
        "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n"
    );
    if line_count > 0 {
        diff.push_str(&format!("@@ -0,0 +1,{line_count} @@\n"));
        for line in text.lines() {
            diff.push('+');
            diff.push_str(line);
            diff.push('\n');
        }
    }
    diff
}

fn parse_git_diff(diff: &str) -> BTreeMap<String, ParsedFileDiff> {
    let mut files = BTreeMap::new();
    for section in git_diff_sections(diff) {
        if let Some(parsed) = parsed_git_section(section) {
            merge_file_diff(&mut files, parsed);
        }
    }
    files
}

fn git_diff_sections(diff: &str) -> Vec<&str> {
    let mut starts = Vec::new();
    for (index, _) in diff.match_indices("diff --git ") {
        starts.push(index);
    }
    if starts.is_empty() {
        return Vec::new();
    }
    starts
        .iter()
        .enumerate()
        .map(|(position, start)| {
            let end = starts.get(position + 1).copied().unwrap_or(diff.len());
            &diff[*start..end]
        })
        .collect()
}

fn parsed_git_section(section: &str) -> Option<ParsedFileDiff> {
    let path = git_section_path(section)?;
    let old_path = git_section_old_path(section);
    let (additions, deletions, hunks) = parse_unified_diff(section);
    Some(ParsedFileDiff {
        path,
        old_path,
        change_type: git_section_change_type(section),
        additions,
        deletions,
        raw_diff: section.trim_end().to_string(),
        hunks,
    })
}

fn git_section_path(section: &str) -> Option<String> {
    let mut old_path = None;
    let mut deleted_file = false;
    let mut rename_to = None;
    for line in section.lines() {
        if let Some(rest) = line.strip_prefix("+++ b/") {
            return Some(rest.to_string());
        }
        if let Some(rest) = line.strip_prefix("--- a/") {
            old_path = Some(rest.to_string());
        }
        if line == "+++ /dev/null" {
            deleted_file = true;
        }
        if let Some(rest) = line.strip_prefix("rename to ") {
            rename_to = Some(rest.to_string());
        }
    }
    if deleted_file {
        return old_path;
    }
    rename_to
}

fn git_section_old_path(section: &str) -> Option<String> {
    section
        .lines()
        .find_map(|line| line.strip_prefix("rename from ").map(str::to_string))
}

fn git_section_change_type(section: &str) -> String {
    if section
        .lines()
        .any(|line| line.starts_with("new file mode"))
    {
        return "create".to_string();
    }
    if section
        .lines()
        .any(|line| line.starts_with("deleted file mode"))
    {
        return "delete".to_string();
    }
    if section.lines().any(|line| line.starts_with("rename from ")) {
        return "rename".to_string();
    }
    "update".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;

    #[test]
    fn parses_unified_diff_counts_and_hunk_lines() {
        let diff = "@@ -1,3 +1,4 @@\n context\n-old\n+new\n+extra\n";
        let (additions, deletions, hunks) = parse_unified_diff(diff);
        assert_eq!(additions, 2);
        assert_eq!(deletions, 1);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0]["lines"][1]["kind"], "delete");
        assert_eq!(hunks[0]["lines"][2]["new_line"], 2);
    }

    #[test]
    fn builds_file_change_summary_content_part() {
        let part = file_change_summary_part(
            "turn-1",
            "turn-1-final",
            &[json!({
                "type": "fileChange",
                "status": "completed",
                "changes": [{
                    "path": "/repo/src/lib.rs",
                    "kind": { "type": "update", "move_path": null },
                    "diff": "@@ -1 +1,2 @@\n-old\n+new\n+extra\n"
                }]
            })],
            Some("/repo"),
        )
        .expect("summary part");
        assert_eq!(part["type"], "file_change_summary");
        assert_eq!(part["files"], 1);
        assert_eq!(part["additions"], 2);
        assert_eq!(part["deletions"], 1);
        assert_eq!(part["files_summary"][0]["path"], "src/lib.rs");
        assert_eq!(
            part["diff_bundle"]["files"][0]["raw_diff"]
                .as_str()
                .unwrap()
                .len()
                > 0,
            true
        );
    }

    #[test]
    fn parses_deleted_git_file_path_and_deletions() {
        let files = parse_git_diff(
            "diff --git a/old.swift b/old.swift\n\
             deleted file mode 100644\n\
             index 1111111..0000000\n\
             --- a/old.swift\n\
             +++ /dev/null\n\
             @@ -1,2 +0,0 @@\n\
             -one\n\
             -two\n",
        );
        let file = files.get("old.swift").expect("deleted file summary");
        assert_eq!(file.change_type, "delete");
        assert_eq!(file.additions, 0);
        assert_eq!(file.deletions, 2);
    }

    #[tokio::test]
    async fn branch_changes_includes_deleted_and_untracked_files() {
        let repo = test_repo("branch-changes");
        std::fs::create_dir_all(&repo).unwrap();
        run_git(&repo, &["init"]);
        run_git(&repo, &["config", "user.name", "Niuma Test"]);
        run_git(&repo, &["config", "user.email", "niuma@example.test"]);
        std::fs::write(repo.join("tracked.txt"), "one\ntwo\n").unwrap();
        run_git(&repo, &["add", "tracked.txt"]);
        run_git(&repo, &["commit", "-m", "initial"]);

        std::fs::remove_file(repo.join("tracked.txt")).unwrap();
        std::fs::write(repo.join("new.txt"), "alpha\nbeta\n").unwrap();

        let changes = branch_changes(repo.to_str().unwrap(), None).await.unwrap();
        assert_eq!(changes.summary["files"], 2);
        assert_eq!(changes.summary["additions"], 2);
        assert_eq!(changes.summary["deletions"], 2);
        assert!(changes.files_summary.to_string().contains("new.txt"));
        assert!(changes.files_summary.to_string().contains("tracked.txt"));

        std::fs::remove_dir_all(repo).unwrap();
    }

    fn test_repo(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("niuma-{label}-{}", uuid::Uuid::new_v4()))
    }

    fn run_git(repo: &Path, args: &[&str]) {
        let status = StdCommand::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {:?} failed", args);
    }
}
