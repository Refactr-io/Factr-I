//! `projects.tree` / `projects.list` / `projects.project_sessions` / `projects.record_repos`, served
//! from the engine's own sessions grouped by working directory (project = git repo root, or the
//! directory itself outside git; sessions with no usable directory fall into the "Home" bucket).
//! Factr's `state.db` never holds engine chats, so forwarding these showed an empty sidebar.
//! Ids and lane keys follow the desktop's persisted-state contract (`tui_gateway/project_tree.py`).

use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(super) const NO_PROJECT_ID: &str = "__no_project__";

/// Where a working directory lives: the repo's main root, this checkout's root, and the branch.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Checkout {
    pub repo_root: String,
    pub worktree_root: String,
    pub branch: String,
}

/// Walk up from `cwd` to the nearest `.git`; a linked worktree folds under its main repo.
pub(super) fn resolve_git(cwd: &str) -> Option<Checkout> {
    for dir in Path::new(cwd).ancestors() {
        let git = dir.join(".git");
        let here = dir.to_string_lossy().into_owned();
        if git.is_dir() {
            return Some(Checkout { repo_root: here.clone(), worktree_root: here, branch: head_branch(&git) });
        }
        if git.is_file() {
            let text = std::fs::read_to_string(&git).ok()?;
            let gitdir = PathBuf::from(text.strip_prefix("gitdir:")?.trim());
            let gitdir = if gitdir.is_absolute() { gitdir } else { dir.join(gitdir) };
            let main = gitdir.to_string_lossy().split_once("/.git/worktrees/").map(|(m, _)| m.to_string());
            return Some(Checkout { repo_root: main.unwrap_or_else(|| here.clone()), worktree_root: here, branch: head_branch(&gitdir) });
        }
    }
    None
}

fn head_branch(gitdir: &Path) -> String {
    std::fs::read_to_string(gitdir.join("HEAD"))
        .ok()
        .and_then(|h| h.trim().strip_prefix("ref: refs/heads/").map(str::to_owned))
        .unwrap_or_default()
}

fn base_name(path: &str) -> String {
    path.trim_end_matches('/').rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(path).to_string()
}

struct Bucket {
    label: String,
    path: Option<String>,
    /// lane id -> (label, path, is_main, session rows)
    lanes: BTreeMap<String, (String, Option<String>, bool, Vec<Value>)>,
    repo_label: String,
}

/// Build the tree from desktop-shaped session rows (`map::session_info`, each with `cwd`).
/// `resolve` maps a directory to its checkout (`None`: not git); `exists` says whether it is still there.
pub(super) fn build_tree(
    sessions: &[Value],
    home: &str,
    preview_limit: usize,
    hydrate: bool,
    resolve: &dyn Fn(&str) -> Option<Checkout>,
    exists: &dyn Fn(&str) -> bool,
) -> Value {
    let mut projects: BTreeMap<String, Bucket> = BTreeMap::new();
    for s in sessions {
        let cwd = s["cwd"].as_str().unwrap_or("").trim().trim_end_matches('/');
        let mut row = s.clone();
        let (pid, lane_id, lane_label, lane_path, is_main, label, path) =
            if cwd.is_empty() || cwd == home.trim_end_matches('/') || !exists(cwd) {
                (NO_PROJECT_ID.to_string(), NO_PROJECT_ID.to_string(), "Home".to_string(), None, true, "Home".to_string(), None)
            } else if let Some(c) = resolve(cwd) {
                row["git_repo_root"] = json!(c.repo_root);
                row["git_branch"] = json!(c.branch);
                if c.worktree_root == c.repo_root {
                    let lane = format!("{}::branch::{}", c.repo_root, c.branch);
                    let lane_label = if c.branch.is_empty() { "main".to_string() } else { c.branch.clone() };
                    (c.repo_root.clone(), lane, lane_label, Some(c.repo_root.clone()), true, base_name(&c.repo_root), Some(c.repo_root))
                } else {
                    (c.repo_root.clone(), c.worktree_root.clone(), base_name(&c.worktree_root), Some(c.worktree_root), false, base_name(&c.repo_root), Some(c.repo_root))
                }
            } else {
                let lane = format!("{cwd}::branch::");
                (cwd.to_string(), lane, "main".to_string(), Some(cwd.to_string()), true, base_name(cwd), Some(cwd.to_string()))
            };
        let bucket = projects.entry(pid).or_insert_with(|| Bucket { repo_label: label.clone(), label, path, lanes: BTreeMap::new() });
        bucket.lanes.entry(lane_id).or_insert_with(|| (lane_label, lane_path, is_main, Vec::new())).3.push(row);
    }
    let last = |r: &Value| r["last_active"].as_f64().unwrap_or(0.0);
    let mut out: Vec<(f64, Value)> = Vec::new();
    let mut scoped: Vec<String> = Vec::new();
    for (id, b) in projects {
        let home_bucket = id == NO_PROJECT_ID;
        let mut all: Vec<Value> = b.lanes.values().flat_map(|l| l.3.iter().cloned()).collect();
        all.sort_by(|a, c| last(c).total_cmp(&last(a)));
        let ids: Vec<Value> = all.iter().map(|r| r["id"].clone()).collect();
        if !home_bucket {
            scoped.extend(all.iter().filter_map(|r| r["id"].as_str().map(str::to_owned)));
        }
        let lanes: Vec<Value> = b.lanes.into_iter().map(|(lane_id, (label, path, is_main, mut rows))| {
            rows.sort_by(|a, c| last(c).total_cmp(&last(a)));
            let mut lane = json!({ "id": lane_id, "label": label, "path": path, "sessions": if hydrate { rows } else { Vec::new() } });
            if !home_bucket {
                lane["isMain"] = json!(is_main);
                lane["isHome"] = json!(is_main);
            }
            lane
        }).collect();
        let tokens: u64 = all.iter().map(|r| r["input_tokens"].as_u64().unwrap_or(0) + r["output_tokens"].as_u64().unwrap_or(0)).sum();
        let newest = all.first().map(last).unwrap_or(0.0);
        let count = all.len();
        let mut project = json!({
            "id": id, "label": b.label, "path": b.path, "isAuto": !home_bucket, "archived": false,
            "repos": [{ "id": id, "label": b.repo_label, "path": b.path, "groups": lanes, "sessionCount": count }],
            "sessionCount": count, "totalTokens": tokens, "totalCostUsd": 0, "lastActive": newest,
            "previewSessions": all.into_iter().take(preview_limit).collect::<Vec<_>>(), "sessionIds": ids,
        });
        if home_bucket {
            project["isNoProject"] = json!(true);
        }
        out.push((newest, project));
    }
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    json!({ "projects": out.into_iter().map(|p| p.1).collect::<Vec<_>>(), "scoped_session_ids": scoped })
}

/// The RPC bodies, from the live sessions list.
pub(super) fn tree_reply(sessions: &[Value], home: &str, params: &Value) -> Value {
    let limit = |k: &str, d: usize| params[k].as_u64().map_or(d, |n| n as usize);
    let mut t = build_tree(sessions, home, limit("preview_limit", 3), false, &resolve_git, &|p| Path::new(p).is_dir());
    t["active_id"] = Value::Null;
    t
}

pub(super) fn project_sessions_reply(sessions: &[Value], home: &str, project_id: &str) -> Value {
    let t = build_tree(sessions, home, 0, true, &resolve_git, &|p| Path::new(p).is_dir());
    json!({ "project": t["projects"].as_array().and_then(|l| l.iter().find(|p| p["id"] == project_id)).cloned() })
}

pub(super) fn record_repos_reply(params: &Value) -> Value {
    json!({
        "repos": [], "accepted": true,
        "discovery_policy": params.get("discovery_policy").cloned().unwrap_or_else(|| json!({ "enabled": false, "roots": [], "exclude_paths": [] })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, cwd: &str, at: f64) -> Value {
        json!({ "id": id, "cwd": cwd, "last_active": at, "input_tokens": 2, "output_tokens": 3 })
    }
    fn git(cwd: &str) -> Option<Checkout> {
        let root = if cwd.starts_with("/w/app") { "/w/app" } else { return None };
        let wt = if cwd.starts_with("/w/app/.worktrees/x") { "/w/app/.worktrees/x" } else { root };
        Some(Checkout { repo_root: root.into(), worktree_root: wt.into(), branch: "dev".into() })
    }

    #[test]
    fn groups_sessions_by_repo_lane_and_home() {
        let s = vec![
            row("a", "/w/app/src", 10.0), row("b", "/w/app/.worktrees/x", 30.0), row("c", "/w/plain", 20.0),
            row("d", "", 5.0), row("e", "/gone", 6.0),
        ];
        let t = build_tree(&s, "/Users/me", 2, true, &git, &|p| p != "/gone");
        let ps = t["projects"].as_array().unwrap();
        let ids: Vec<&str> = ps.iter().map(|p| p["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["/w/app", "/w/plain", NO_PROJECT_ID]);
        let app = &ps[0];
        assert_eq!(app["label"], "app");
        assert_eq!(app["sessionCount"], 2);
        assert_eq!(app["totalTokens"], 10);
        assert_eq!(app["lastActive"], 30.0);
        let lanes: Vec<&str> = app["repos"][0]["groups"].as_array().unwrap().iter().map(|g| g["id"].as_str().unwrap()).collect();
        assert_eq!(lanes, ["/w/app/.worktrees/x", "/w/app::branch::dev"]);
        assert_eq!(app["previewSessions"][0]["id"], "b");
        assert_eq!(ps[1]["repos"][0]["groups"][0]["id"], "/w/plain::branch::");
        assert_eq!(ps[2]["isNoProject"], true);
        assert_eq!(ps[2]["sessionCount"], 2);
        assert_eq!(t["scoped_session_ids"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn tree_lanes_carry_no_rows_but_drill_in_does() {
        let s = vec![row("a", "/w/app", 1.0)];
        let lean = build_tree(&s, "/h", 3, false, &git, &|_| true);
        assert_eq!(lean["projects"][0]["repos"][0]["groups"][0]["sessions"], json!([]));
        let full = build_tree(&s, "/h", 0, true, &git, &|_| true);
        assert_eq!(full["projects"][0]["repos"][0]["groups"][0]["sessions"][0]["git_branch"], "dev");
        assert_eq!(full["projects"][0]["previewSessions"], json!([]));
    }

    #[test]
    fn resolves_real_git_dir_and_linked_worktree() {
        let d = std::env::temp_dir().join(format!("factr-proj-{}", std::process::id()));
        let main = d.join("repo");
        std::fs::create_dir_all(main.join(".git/worktrees/wt")).unwrap();
        std::fs::write(main.join(".git/HEAD"), "ref: refs/heads/feat\n").unwrap();
        std::fs::write(main.join(".git/worktrees/wt/HEAD"), "ref: refs/heads/other\n").unwrap();
        let wt = d.join("wt");
        std::fs::create_dir_all(wt.join("sub")).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", main.join(".git/worktrees/wt").display())).unwrap();
        std::fs::create_dir_all(main.join("a")).unwrap();
        let m = resolve_git(&main.join("a").to_string_lossy()).unwrap();
        assert_eq!((m.repo_root.as_str(), m.branch.as_str()), (main.to_str().unwrap(), "feat"));
        let w = resolve_git(&wt.join("sub").to_string_lossy()).unwrap();
        assert_eq!((w.repo_root.as_str(), w.worktree_root.as_str(), w.branch.as_str()), (main.to_str().unwrap(), wt.to_str().unwrap(), "other"));
        assert!(resolve_git("/definitely/not/here").is_none());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn project_sessions_picks_one_project() {
        let s = vec![row("a", "/w/app", 1.0), row("c", "/w/plain", 2.0)];
        let t = build_tree(&s, "/h", 0, true, &git, &|_| true);
        assert_eq!(t["projects"].as_array().unwrap().len(), 2);
        assert_eq!(record_repos_reply(&json!({}))["accepted"], true);
    }
}
