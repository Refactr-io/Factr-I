//! `projects.tree` / `projects.list` / `projects.project_sessions` / `projects.record_repos`, served
//! from the engine's own sessions grouped by working directory (project = git repo root, or the
//! directory itself outside git; sessions with no usable directory fall into the "Home" bucket).
//! Factr's `state.db` never holds engine chats, so forwarding these showed an empty sidebar.
//! Ids and lane keys follow the desktop's persisted-state contract (`tui_gateway/project_tree.py`).

use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

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

/// A project the user created (a `projects.db` row): always listed, even with no sessions, and it claims
/// every session whose directory sits under one of its folders.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct ExplicitProject {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub board_slug: Option<String>,
    pub primary_path: Option<String>,
    pub archived: bool,
    pub created_at: i64,
    /// (path, label, is_primary, added_at)
    pub folders: Vec<(String, Option<String>, bool, i64)>,
}

impl ExplicitProject {
    fn to_dict(&self) -> Value {
        json!({
            "id": self.id, "slug": self.slug, "name": self.name, "description": self.description, "icon": self.icon,
            "color": self.color, "board_slug": self.board_slug, "primary_path": self.primary_path,
            "archived": self.archived, "created_at": self.created_at,
            "folders": self.folders.iter().map(|(p, l, primary, at)| json!({ "path": p, "label": l, "is_primary": primary, "added_at": at })).collect::<Vec<_>>(),
        })
    }
}

/// Every project in `<home>/projects.db` plus the active id. A missing or unreadable store is "no projects"
/// (the Python backend creates it on the first `projects.create`); it is only ever opened read-only.
pub(super) fn read_explicit(home: &Path) -> (Vec<ExplicitProject>, Option<String>) {
    let db = home.join("projects.db");
    if !db.is_file() {
        return (Vec::new(), None);
    }
    read_explicit_db(&db).unwrap_or_else(|err| {
        eprintln!("factr: could not read {}: {err}", db.display());
        (Vec::new(), None)
    })
}

fn read_explicit_db(db: &Path) -> rusqlite::Result<(Vec<ExplicitProject>, Option<String>)> {
    use rusqlite::{Connection, OpenFlags};
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    conn.busy_timeout(Duration::from_millis(500))?;
    // Older stores lack some columns; a column that is not there reads as NULL.
    let cols: Vec<String> = conn.prepare("PRAGMA table_info(projects)")?.query_map([], |r| r.get::<_, String>(1))?.collect::<Result<_, _>>()?;
    let col = |name: &str| if cols.iter().any(|c| c == name) { name.to_string() } else { format!("NULL AS {name}") };
    let sql = format!(
        "SELECT id, slug, name, {}, {}, {}, {}, {}, created_at, {} FROM projects ORDER BY created_at, id",
        col("description"), col("icon"), col("color"), col("board_slug"), col("primary_path"), col("archived")
    );
    let mut projects: Vec<ExplicitProject> = conn
        .prepare(&sql)?
        .query_map([], |r| {
            Ok(ExplicitProject {
                id: r.get(0)?,
                slug: r.get(1)?,
                name: r.get(2)?,
                description: r.get(3)?,
                icon: r.get(4)?,
                color: r.get(5)?,
                board_slug: r.get(6)?,
                primary_path: r.get(7)?,
                created_at: r.get(8)?,
                archived: r.get::<_, Option<i64>>(9)?.unwrap_or(0) != 0,
                folders: Vec::new(),
            })
        })?
        .collect::<Result<_, _>>()?;
    let mut folders = conn.prepare("SELECT path, label, is_primary, added_at FROM project_folders WHERE project_id = ?1 ORDER BY is_primary DESC, added_at ASC")?;
    for p in &mut projects {
        p.folders = folders
            .query_map([&p.id], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0, r.get(3)?)))?
            .collect::<Result<_, _>>()?;
    }
    let active = conn
        .query_row("SELECT value FROM project_meta WHERE key = 'active_id'", [], |r| r.get::<_, Option<String>>(0))
        .ok()
        .flatten()
        .filter(|s| !s.is_empty());
    Ok((projects, active))
}

fn trim_path(path: &str) -> &str {
    path.trim().trim_end_matches('/')
}

fn is_under(folder: &str, path: &str) -> bool {
    path == folder || path.strip_prefix(folder).is_some_and(|rest| rest.starts_with('/'))
}

/// Where a session lands: its project, the repo node inside it, and the lane inside that.
struct Place {
    project: String,
    project_label: String,
    project_path: Option<String>,
    repo: String,
    repo_label: String,
    repo_path: Option<String>,
    lane: String,
    lane_label: String,
    lane_path: Option<String>,
    is_main: bool,
}

type Lane = (String, Option<String>, bool, Vec<Value>);

struct Repo {
    label: String,
    path: Option<String>,
    lanes: BTreeMap<String, Lane>,
}

struct Bucket {
    label: String,
    path: Option<String>,
    explicit: Option<usize>,
    repos: BTreeMap<String, Repo>,
}

/// The repo node and lane a checkout gives a session (same lane ids as the Python builder).
fn checkout_place(c: &Checkout) -> (String, String, String, Option<String>, bool) {
    if c.worktree_root == c.repo_root {
        let lane = format!("{}::branch::{}", c.repo_root, c.branch);
        let label = if c.branch.is_empty() { "main".to_string() } else { c.branch.clone() };
        (c.repo_root.clone(), lane, label, Some(c.repo_root.clone()), true)
    } else {
        (c.repo_root.clone(), c.worktree_root.clone(), base_name(&c.worktree_root), Some(c.worktree_root.clone()), false)
    }
}

/// Build the tree from desktop-shaped session rows (`map::session_info`, each with `cwd`).
/// `resolve` maps a directory to its checkout (`None`: not git); `exists` says whether it is still there.
/// `explicit` are the user's own projects: they claim sessions by folder (deepest folder wins) and are listed
/// even when empty; every other session is grouped by repo / directory, or falls into "Home".
pub(super) fn build_tree(
    sessions: &[Value],
    home: &str,
    preview_limit: usize,
    hydrate: bool,
    explicit: &[ExplicitProject],
    resolve: &dyn Fn(&str) -> Option<Checkout>,
    exists: &dyn Fn(&str) -> bool,
) -> Value {
    let active: Vec<(usize, &ExplicitProject)> = explicit.iter().enumerate().filter(|(_, p)| !p.archived).collect();
    // Declared folder -> owning project; the first project keeps a folder two projects share.
    let mut folders: Vec<(&str, usize)> = Vec::new();
    for (i, p) in &active {
        for (path, ..) in &p.folders {
            let f = trim_path(path);
            if !f.is_empty() && !folders.iter().any(|(g, _)| *g == f) {
                folders.push((f, *i));
            }
        }
    }
    let project_path = |p: &ExplicitProject| {
        p.primary_path.clone().filter(|s| !s.trim().is_empty()).or_else(|| p.folders.first().map(|f| f.0.clone()))
    };
    // The deepest declared folder covering the directory or its repo root; the directory wins a tie.
    let owner = |cwd: &str, repo_root: Option<&str>| -> Option<(usize, &str)> {
        let mut best: Option<(usize, &str)> = None;
        for target in [Some(cwd), repo_root].into_iter().flatten() {
            for (f, i) in &folders {
                if is_under(f, target) && best.is_none_or(|(_, b)| f.len() > b.len()) {
                    best = Some((*i, *f));
                }
            }
        }
        best
    };
    let mut projects: BTreeMap<String, Bucket> = BTreeMap::new();
    for s in sessions {
        let cwd = s["cwd"].as_str().unwrap_or("").trim().trim_end_matches('/');
        let mut row = s.clone();
        let checkout = if cwd.is_empty() { None } else { resolve(cwd) };
        let repo_hint = checkout.as_ref().map(|c| c.repo_root.clone()).or_else(|| s["git_repo_root"].as_str().map(|r| trim_path(r).to_string()).filter(|r| !r.is_empty()));
        let place = if let Some((i, folder)) = (!cwd.is_empty()).then(|| owner(cwd, repo_hint.as_deref())).flatten() {
            let p = &explicit[i];
            let (repo, lane, lane_label, lane_path, is_main) = match &checkout {
                Some(c) => {
                    row["git_repo_root"] = json!(c.repo_root);
                    row["git_branch"] = json!(c.branch);
                    checkout_place(c)
                }
                None => (folder.to_string(), format!("{folder}::branch::"), "main".to_string(), Some(folder.to_string()), true),
            };
            Place {
                project: p.id.clone(),
                project_label: if p.name.is_empty() { p.id.clone() } else { p.name.clone() },
                project_path: project_path(p),
                repo_label: base_name(&repo),
                repo_path: Some(repo.clone()),
                repo,
                lane,
                lane_label,
                lane_path,
                is_main,
            }
        } else if cwd.is_empty() || cwd == home.trim_end_matches('/') || !exists(cwd) {
            home_place()
        } else if let Some(c) = &checkout {
            row["git_repo_root"] = json!(c.repo_root);
            row["git_branch"] = json!(c.branch);
            let (repo, lane, lane_label, lane_path, is_main) = checkout_place(c);
            Place {
                project: repo.clone(),
                project_label: base_name(&repo),
                project_path: Some(repo.clone()),
                repo_label: base_name(&repo),
                repo_path: Some(repo.clone()),
                repo,
                lane,
                lane_label,
                lane_path,
                is_main,
            }
        } else {
            Place {
                project: cwd.to_string(),
                project_label: base_name(cwd),
                project_path: Some(cwd.to_string()),
                repo: cwd.to_string(),
                repo_label: base_name(cwd),
                repo_path: Some(cwd.to_string()),
                lane: format!("{cwd}::branch::"),
                lane_label: "main".to_string(),
                lane_path: Some(cwd.to_string()),
                is_main: true,
            }
        };
        let explicit_idx = explicit.iter().position(|p| p.id == place.project && !p.archived);
        let bucket = projects.entry(place.project).or_insert_with(|| Bucket { label: place.project_label, path: place.project_path, explicit: explicit_idx, repos: BTreeMap::new() });
        let repo = bucket.repos.entry(place.repo).or_insert_with(|| Repo { label: place.repo_label, path: place.repo_path, lanes: BTreeMap::new() });
        repo.lanes.entry(place.lane).or_insert_with(|| (place.lane_label, place.lane_path, place.is_main, Vec::new())).3.push(row);
    }
    // A declared folder shows as a repo even before its first session, and a project with no session at all
    // is still listed: that is how a freshly created project appears.
    for (i, p) in &active {
        let bucket = projects.entry(p.id.clone()).or_insert_with(|| Bucket {
            label: if p.name.is_empty() { p.id.clone() } else { p.name.clone() },
            path: project_path(p),
            explicit: Some(*i),
            repos: BTreeMap::new(),
        });
        for (path, ..) in &p.folders {
            let f = trim_path(path);
            if f.is_empty() {
                continue;
            }
            let root = resolve(f).map_or_else(|| f.to_string(), |c| c.repo_root);
            bucket.repos.entry(root.clone()).or_insert_with(|| Repo { label: base_name(&root), path: Some(root), lanes: BTreeMap::new() });
        }
    }
    let last = |r: &Value| r["last_active"].as_f64().unwrap_or(0.0);
    let mut out: Vec<(f64, Value)> = Vec::new();
    let mut scoped: Vec<String> = Vec::new();
    for (id, b) in projects {
        let home_bucket = id == NO_PROJECT_ID;
        let mut all: Vec<Value> = b.repos.values().flat_map(|r| r.lanes.values().flat_map(|l| l.3.iter().cloned())).collect();
        all.sort_by(|a, c| last(c).total_cmp(&last(a)));
        let ids: Vec<Value> = all.iter().map(|r| r["id"].clone()).collect();
        if !home_bucket {
            scoped.extend(all.iter().filter_map(|r| r["id"].as_str().map(str::to_owned)));
        }
        let repos: Vec<Value> = b.repos.into_iter().map(|(repo_id, repo)| {
            let count: usize = repo.lanes.values().map(|l| l.3.len()).sum();
            let lanes: Vec<Value> = repo.lanes.into_iter().map(|(lane_id, (label, path, is_main, mut rows))| {
                rows.sort_by(|a, c| last(c).total_cmp(&last(a)));
                let mut lane = json!({ "id": lane_id, "label": label, "path": path, "sessions": if hydrate { rows } else { Vec::new() } });
                if !home_bucket {
                    lane["isMain"] = json!(is_main);
                    lane["isHome"] = json!(is_main);
                }
                lane
            }).collect();
            json!({ "id": repo_id, "label": repo.label, "path": repo.path, "groups": lanes, "sessionCount": count })
        }).collect();
        let tokens: u64 = all.iter().map(|r| r["input_tokens"].as_u64().unwrap_or(0) + r["output_tokens"].as_u64().unwrap_or(0)).sum();
        let newest = all.first().map(last).unwrap_or(0.0);
        let count = all.len();
        let mut project = json!({
            "id": id, "label": b.label, "path": b.path, "isAuto": !home_bucket && b.explicit.is_none(), "archived": false,
            "repos": repos,
            "sessionCount": count, "totalTokens": tokens, "totalCostUsd": 0, "lastActive": newest,
            "previewSessions": all.into_iter().take(preview_limit).collect::<Vec<_>>(), "sessionIds": ids,
        });
        if let Some(p) = b.explicit.map(|i| &explicit[i]) {
            project["color"] = json!(p.color);
            project["icon"] = json!(p.icon);
        }
        if home_bucket {
            project["isNoProject"] = json!(true);
        }
        out.push((newest, project));
    }
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    json!({ "projects": out.into_iter().map(|p| p.1).collect::<Vec<_>>(), "scoped_session_ids": scoped })
}

fn home_place() -> Place {
    Place {
        project: NO_PROJECT_ID.into(),
        project_label: "Home".into(),
        project_path: None,
        repo: NO_PROJECT_ID.into(),
        repo_label: "Home".into(),
        repo_path: None,
        lane: NO_PROJECT_ID.into(),
        lane_label: "Home".into(),
        lane_path: None,
        is_main: true,
    }
}

/// The RPC bodies, from the live sessions list and `projects.db`.
pub(super) fn tree_reply(sessions: &[Value], home: &str, params: &Value, explicit: &[ExplicitProject], active: Option<String>) -> Value {
    let limit = |k: &str, d: usize| params[k].as_u64().map_or(d, |n| n as usize);
    let mut t = build_tree(sessions, home, limit("preview_limit", 3), false, explicit, &resolve_git, &|p| Path::new(p).is_dir());
    t["active_id"] = json!(active);
    t
}

pub(super) fn project_sessions_reply(sessions: &[Value], home: &str, project_id: &str, explicit: &[ExplicitProject]) -> Value {
    let t = build_tree(sessions, home, 0, true, explicit, &resolve_git, &|p| Path::new(p).is_dir());
    json!({ "project": t["projects"].as_array().and_then(|l| l.iter().find(|p| p["id"] == project_id)).cloned() })
}

/// `projects.list`: every project (archived ones too) and the active id, as the Python backend answers it.
pub(super) fn list_reply(explicit: &[ExplicitProject], active: Option<String>) -> Value {
    json!({ "projects": explicit.iter().map(ExplicitProject::to_dict).collect::<Vec<_>>(), "active_id": active })
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
        let t = build_tree(&s, "/Users/me", 2, true, &[], &git, &|p| p != "/gone");
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
        let lean = build_tree(&s, "/h", 3, false, &[], &git, &|_| true);
        assert_eq!(lean["projects"][0]["repos"][0]["groups"][0]["sessions"], json!([]));
        let full = build_tree(&s, "/h", 0, true, &[], &git, &|_| true);
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
        let t = build_tree(&s, "/h", 0, true, &[], &git, &|_| true);
        assert_eq!(t["projects"].as_array().unwrap().len(), 2);
        assert_eq!(record_repos_reply(&json!({}))["accepted"], true);
    }

    const ICLOUD: &str = "/Users/me/Library/Mobile Documents/com~apple~CloudDocs/Refactr";

    fn explicit(id: &str, name: &str, folders: &[&str]) -> ExplicitProject {
        ExplicitProject {
            id: id.into(),
            slug: name.to_lowercase(),
            name: name.into(),
            primary_path: folders.first().map(|f| f.to_string()),
            folders: folders.iter().enumerate().map(|(n, f)| (f.to_string(), None, n == 0, 1)).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_project_with_no_sessions_is_still_listed() {
        // The bug: only session-derived projects were listed, so a project created from the sidebar never showed.
        let s = vec![row("a", "/w/plain", 5.0)];
        let t = build_tree(&s, "/Users/me", 3, false, &[explicit("p_297a2943", "Refactr", &[ICLOUD])], &git, &|_| true);
        let ps = t["projects"].as_array().unwrap();
        let p = ps.iter().find(|p| p["id"] == "p_297a2943").expect("the new project is in the tree");
        assert_eq!((p["label"].as_str(), p["isAuto"].as_bool(), p["sessionCount"].as_u64()), (Some("Refactr"), Some(false), Some(0)));
        assert_eq!(p["path"], ICLOUD);
        assert_eq!(p["repos"][0]["path"], ICLOUD);
        assert_eq!(p["repos"][0]["groups"], json!([]));
        assert!(ps.iter().any(|p| p["id"] == "/w/plain"), "auto projects stay");
    }

    #[test]
    fn a_project_claims_sessions_under_its_folders() {
        let inside = format!("{ICLOUD}/notes");
        let s = vec![row("a", &inside, 9.0), row("b", "/w/app/src", 8.0), row("c", "/w/other", 7.0), row("d", "/w/ap", 6.0)];
        let mine = explicit("p_1", "Mine", &[ICLOUD, "/w/app"]);
        let t = build_tree(&s, "/Users/me", 3, true, &[mine], &git, &|_| true);
        let ps = t["projects"].as_array().unwrap();
        let p = ps.iter().find(|p| p["id"] == "p_1").unwrap();
        assert_eq!(p["sessionCount"], 2);
        assert_eq!(p["sessionIds"], json!(["a", "b"]));
        let repos: Vec<&str> = p["repos"].as_array().unwrap().iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(repos, [ICLOUD, "/w/app"]);
        let lane = &p["repos"][0]["groups"][0];
        assert_eq!((lane["id"].as_str(), lane["sessions"][0]["id"].as_str()), (Some(format!("{ICLOUD}::branch::").as_str()), Some("a")));
        assert_eq!(p["repos"][1]["groups"][0]["id"], "/w/app::branch::dev");
        // Not under a declared folder (a sibling sharing a name prefix included): not claimed.
        assert!(ps.iter().any(|p| p["id"] == "/w/other") && ps.iter().any(|p| p["id"] == "/w/ap"));
        assert_eq!(t["scoped_session_ids"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn the_deepest_folder_wins_and_archived_projects_are_hidden() {
        let s = vec![row("a", "/w/app/src", 1.0)];
        let outer = explicit("p_out", "Outer", &["/w"]);
        let inner = explicit("p_in", "Inner", &["/w/app"]);
        let mut gone = explicit("p_gone", "Gone", &["/w/app/src"]);
        gone.archived = true;
        let t = build_tree(&s, "/h", 3, false, &[outer, inner, gone], &git, &|_| true);
        let count = |id: &str| t["projects"].as_array().unwrap().iter().find(|p| p["id"] == id).map(|p| p["sessionCount"].clone());
        assert_eq!(count("p_in"), Some(json!(1)));
        assert_eq!(count("p_out"), Some(json!(0)));
        assert_eq!(count("p_gone"), None);
    }

    fn temp_home(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("factr-projdb-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reads_projects_written_by_the_backend() {
        let d = temp_home("read");
        assert_eq!(read_explicit(&d), (Vec::new(), None), "no store yet is no projects");
        let conn = rusqlite::Connection::open(d.join("projects.db")).unwrap();
        conn.execute_batch(&format!(
            "CREATE TABLE projects (id TEXT PRIMARY KEY, slug TEXT NOT NULL UNIQUE, name TEXT NOT NULL, description TEXT, icon TEXT,
               color TEXT, board_slug TEXT, primary_path TEXT, created_at INTEGER NOT NULL, archived INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE project_folders (project_id TEXT NOT NULL, path TEXT NOT NULL, label TEXT, is_primary INTEGER NOT NULL DEFAULT 0,
               added_at INTEGER NOT NULL, PRIMARY KEY (project_id, path));
             CREATE TABLE project_meta (key TEXT PRIMARY KEY, value TEXT);
             INSERT INTO projects (id, slug, name, primary_path, created_at) VALUES ('p_297a2943', 'refactr', 'Refactr', '{ICLOUD}', 1791729726);
             INSERT INTO projects (id, slug, name, created_at, archived) VALUES ('p_old', 'old', 'Old', 1, 1);
             INSERT INTO project_folders VALUES ('p_297a2943', '{ICLOUD}', NULL, 1, 1791729726);
             INSERT INTO project_meta VALUES ('active_id', 'p_297a2943');"
        ))
        .unwrap();
        drop(conn);
        let (projects, active) = read_explicit(&d);
        assert_eq!(active.as_deref(), Some("p_297a2943"));
        assert_eq!(projects.len(), 2);
        assert_eq!(projects[1].folders, vec![(ICLOUD.to_string(), None, true, 1791729726)]);
        let list = list_reply(&projects, active);
        assert_eq!(list["projects"][1]["name"], "Refactr");
        assert_eq!(list["projects"][0]["archived"], true);
        assert_eq!(list["projects"][1]["folders"][0]["is_primary"], true);
        assert_eq!(list["active_id"], "p_297a2943");
        let tree = tree_reply(&[], "/Users/me", &json!({}), &projects, Some("p_297a2943".into()));
        assert_eq!(tree["active_id"], "p_297a2943");
        let ids: Vec<&str> = tree["projects"].as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["p_297a2943"]);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_unreadable_store_is_no_projects_not_an_error() {
        let d = temp_home("bad");
        std::fs::write(d.join("projects.db"), "not a database").unwrap();
        assert_eq!(read_explicit(&d), (Vec::new(), None));
        let _ = std::fs::remove_dir_all(d);
    }
}
