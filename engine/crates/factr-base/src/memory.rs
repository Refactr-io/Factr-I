//! Memory system for cross-session learning
//!
//! Provides persistent memory that survives across sessions, organized by:
//! - Project (per working directory)
//! - Global (user-level preferences)
//!
//! Jev provides typed relevance decisions. Optional text-generating extraction
//! is independent of recall and is never required to read existing memories.

use crate::memory_graph::{GRAPH_VERSION, MemoryGraph};
use crate::memory_types::ranking::top_k_by_ord;
use crate::storage;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

mod cache;
#[path = "memory/learned.rs"]
pub mod learned;
#[path = "memory/pending.rs"]
mod pending;
#[path = "memory_prompt.rs"]
mod prompt_support;

pub use crate::memory_types::{
    LearnedMeta, MemoryCategory, MemoryEntry, MemoryScope, MemoryStore, Reinforcement, TrustLevel,
    format_relevant_display_prompt, format_relevant_prompt,
};
use crate::memory_types::{
    format_entries_for_prompt, memory_matches_search,
    normalize_memory_search_text, normalize_search_text,
};
use cache::{cache_graph, cached_graph, forget_graph, with_cached};
pub(crate) use pending::set_pending_memory_for_project_with_selection;
pub use pending::{
    PendingMemory, clear_all_injected_memories, clear_all_pending_memory, clear_injected_memories,
    clear_pending_memory, has_any_pending_memory, has_pending_memory, is_memory_injected,
    is_memory_injected_any, mark_memories_injected, mark_memories_known, pin_known, set_pending_memory,
    set_pending_memory_for_project, set_pending_memory_with_ids,
    set_pending_memory_with_ids_and_display, sync_injected_memories, take_pending_memory,
    take_pending_memory_for_project, unpin_known,
};
#[cfg(test)]
use pending::{backdate_injected_memory_for_test, insert_pending_memory_for_test};
pub use prompt_support::{
    focus_query_text, format_context_for_relevance, format_focused_query_for_relevance,
};

const LEGACY_NOTE_CATEGORY: &str = "note";

/// Producer of synthetic [`MemoryEntry`] values contributed by a higher layer.
///
/// Used to invert the legacy `memory -> skill` dependency: the `skill` layer
/// (which already depends on `MemoryEntry`) registers a provider that turns the
/// shared skill registry into synthetic memory entries, instead of `memory`
/// reaching up into `skill::SkillRegistry`.
type SyntheticEntryProvider = fn() -> Vec<MemoryEntry>;

static SYNTHETIC_ENTRY_PROVIDERS: std::sync::LazyLock<
    std::sync::RwLock<Vec<SyntheticEntryProvider>>,
> = std::sync::LazyLock::new(|| std::sync::RwLock::new(Vec::new()));

/// Register a provider of synthetic memory entries (e.g. skills).
///
/// Inverts `memory -> skill`: higher layers register their synthetic-entry
/// source here at startup so `memory` stays free of upward references.
pub fn register_synthetic_entry_provider(provider: SyntheticEntryProvider) {
    SYNTHETIC_ENTRY_PROVIDERS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(provider);
}

#[cfg(test)]
fn collect_synthetic_entries() -> Vec<MemoryEntry> {
    let providers = SYNTHETIC_ENTRY_PROVIDERS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut entries = Vec::new();
    for provider in providers.iter() {
        entries.extend(provider());
    }
    entries
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct LegacyNotesFile {
    #[serde(default)]
    entries: Vec<LegacyNoteEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyNoteEntry {
    id: String,
    content: String,
    created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
}

pub type MemoryEventSink = Arc<dyn Fn(crate::protocol::ServerEvent) + Send + Sync>;



#[derive(Debug, Clone)]
pub struct MemoryManager {
    project_dir: Option<PathBuf>,
    /// When true, use isolated test storage instead of real memory
    test_mode: bool,
    include_skills: bool,
}

/// Recall output retains the exact entries supplied to the relevance judge so
/// publication can reject even non-rendered metadata changes during inference.
#[derive(Default)]
pub struct MemoryRelevanceResult {
    pub prompt: Option<String>,
    pub display_prompt: Option<String>,
    pub selected_entries: Vec<MemoryEntry>,
}

/// Result of a local recall with the numbers behind it (for the `memory.recall` span).
#[derive(Default)]
pub struct LocalRecall {
    pub terms: Vec<String>,
    pub candidates: usize,
    pub entries: Vec<MemoryEntry>,
    pub suppressed: usize,
}

impl MemoryManager {
    pub fn new() -> Self {
        Self {
            project_dir: None,
            test_mode: false,
            include_skills: true,
        }
    }

    pub fn with_project_dir(mut self, project_dir: impl Into<PathBuf>) -> Self {
        self.project_dir = Some(project_dir.into());
        self
    }

    pub fn with_skills(mut self, include_skills: bool) -> Self {
        self.include_skills = include_skills;
        self
    }

    /// Create a memory manager in test mode (isolated storage)
    pub fn new_test() -> Self {
        Self {
            project_dir: None,
            test_mode: true,
            include_skills: true,
        }
    }

    /// Check if running in test mode
    pub fn is_test_mode(&self) -> bool {
        self.test_mode
    }

    /// Set test mode (for debug sessions)
    pub fn set_test_mode(&mut self, test_mode: bool) {
        self.test_mode = test_mode;
    }

    /// Clear all test memories (only works in test mode)
    pub fn clear_test_storage(&self) -> Result<()> {
        if !self.test_mode {
            anyhow::bail!("clear_test_storage only allowed in test mode");
        }

        let test_dir = storage::factr_dir()?.join("memory").join("test");
        crate::memory_store::close(&self.db_path()?);
        if test_dir.exists() {
            std::fs::remove_dir_all(&test_dir)?;
            crate::logging::info("Cleared test memory storage");
        }
        Ok(())
    }

    fn get_project_dir(&self) -> Option<PathBuf> {
        self.project_dir.clone()
    }

    /// The engine's memory database (`factr.db`), or a throwaway one in tests.
    pub(crate) fn db_path(&self) -> Result<PathBuf> {
        Ok(if self.test_mode {
            storage::factr_dir()?.join("memory").join("test").join("memory.db")
        } else {
            storage::factr_dir()?.join("factr.db")
        })
    }

    /// `project:<hash of the project dir>` (the old JSON file's name), if any.
    ///
    /// The hash is `DefaultHasher` (see `learned::project_scope`), whose algorithm std does not
    /// promise to keep across Rust releases. A change would orphan every project scope, and it cannot
    /// be migrated: only the hash is stored, never the directory. `project_scope_hash_is_pinned`
    /// fails if a toolchain bump changes it; the fix then is to keep the old algorithm inline.
    fn project_scope(&self) -> Option<String> {
        if self.test_mode {
            return Some("project:test".to_string());
        }
        let project_dir = self.get_project_dir()?;
        Some(learned::project_scope(&project_dir.to_string_lossy()))
    }

    fn legacy_notes_path(&self) -> Result<Option<PathBuf>> {
        if self.test_mode {
            let test_dir = storage::factr_dir()?.join("notes").join("test");
            std::fs::create_dir_all(&test_dir)?;
            return Ok(Some(test_dir.join("test_notes.json")));
        }

        let project_dir = match self.get_project_dir() {
            Some(d) => d,
            None => return Ok(None),
        };

        let project_hash = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            project_dir.hash(&mut hasher);
            format!("{:016x}", hasher.finish())
        };

        Ok(Some(
            storage::factr_dir()?
                .join("notes")
                .join(format!("{}.json", project_hash)),
        ))
    }

    fn normalize_graph_search_text(graph: &mut MemoryGraph) -> bool {
        let mut changed = false;
        for memory in graph.memories.values_mut() {
            let expected = normalize_memory_search_text(&memory.content, &memory.tags);
            if memory.search_text != expected {
                memory.search_text = expected;
                changed = true;
            }
        }
        changed
    }

    fn import_legacy_notes_into_graph(&self, graph: &mut MemoryGraph) -> Result<bool> {
        let Some(path) = self.legacy_notes_path()? else {
            return Ok(false);
        };
        if !path.exists() {
            return Ok(false);
        }

        let legacy: LegacyNotesFile = storage::read_json(&path)?;
        if legacy.entries.is_empty() {
            return Ok(false);
        }

        let mut changed = false;
        for note in legacy.entries {
            if graph.memories.contains_key(&note.id) {
                continue;
            }

            let mut entry = MemoryEntry::new(
                MemoryCategory::Custom(LEGACY_NOTE_CATEGORY.to_string()),
                note.content,
            );
            entry.id = note.id;
            entry.created_at = note.created_at;
            entry.updated_at = note.created_at;
            entry.source = Some("legacy_remember_migration".to_string());
            if let Some(tag) = note.tag {
                entry.tags.push(tag);
            }
            graph.add_memory(entry);
            changed = true;
        }

        Ok(changed)
    }

    /// Exact duplicates reinforce an existing
    /// entry only within the requested scope, never mutate a different project.
    pub fn remember_project(&self, entry: MemoryEntry) -> Result<String> {
        crate::memory_quality::gate_entry(&entry)?;
        Ok(self.remember_project_outcome(entry)?.id().to_string())
    }

    fn remember_project_outcome(&self, entry: MemoryEntry) -> Result<crate::memory_store::Remembered> {
        anyhow::ensure!(
            self.project_scope().is_some(),
            "Project memory requires a working directory; use global scope explicitly"
        );
        // Loading first imports any legacy notes for this project (once) into the store.
        self.load_project_graph()?;
        let scope = self.project_scope().expect("checked above");
        let db = self.db_path()?;
        let outcome = crate::memory_store::remember(&db, &scope, entry)?;
        forget_graph(&format!("{}#{scope}", db.display()));
        forget_graph(&format!("{}#global", db.display()));
        Ok(outcome)
    }

    /// Writes just this memory's row: rewriting the whole graph lost a concurrent writer's rows.
    pub fn remember_global(&self, entry: MemoryEntry) -> Result<String> {
        crate::memory_quality::gate_entry(&entry)?;
        Ok(self.remember_global_outcome(entry)?.id().to_string())
    }

    fn remember_global_outcome(&self, entry: MemoryEntry) -> Result<crate::memory_store::Remembered> {
        let db = self.db_path()?;
        if !self.test_mode {
            self.import_json_once(&db)?;
        }
        let outcome = crate::memory_store::remember(&db, "global", entry)?;
        forget_graph(&format!("{}#global", db.display()));
        Ok(outcome)
    }

    /// Where automatic extraction writes: project scope, or global when there is no working directory.
    pub(crate) fn remember_extracted(&self, entry: MemoryEntry) -> Result<crate::memory_store::Remembered> {
        if self.project_scope().is_some() {
            self.remember_project_outcome(entry)
        } else {
            self.remember_global_outcome(entry)
        }
    }

    /// Active memories (project and global) sharing words with `text`, best match first, for the
    /// extraction prompt's "already known" list, plus the notes `session` learned for itself. Not
    /// filtered by the recall term floor.
    pub(crate) fn related_to(&self, text: &str, limit: usize, session: Option<&str>) -> Result<Vec<MemoryEntry>> {
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for term in crate::memory_recall::local_terms(text) {
            *counts.entry(term).or_default() += 1;
        }
        let mut terms: Vec<(String, usize)> = counts.into_iter().collect();
        terms.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let terms: Vec<String> = terms.into_iter().take(48).map(|(t, _)| t).collect();
        let mut scopes = vec!["global".to_string()];
        scopes.extend(self.project_scope());
        scopes.extend(session.map(|s| format!("session:{s}")));
        let db = self.db_path()?;
        if !self.test_mode {
            self.import_json_once(&db)?;
        }
        crate::memory_store::search(&db, &scopes, &terms, limit)
    }

    /// Small per-database bookkeeping value (`memory_meta`), e.g. `extracted_through:<session>`.
    pub(crate) fn meta_get(&self, key: &str) -> Result<Option<String>> {
        crate::memory_store::meta_get(&self.db_path()?, key)
    }

    pub(crate) fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        crate::memory_store::meta_set(&self.db_path()?, key, value)
    }

    /// Insert or update a memory with a stable ID in the project graph.
    /// Preserves existing inbound/outbound graph relationships while refreshing
    /// content and tags.
    pub fn upsert_project_memory(&self, entry: MemoryEntry) -> Result<String> {
        crate::memory_quality::gate_entry(&entry)?;
        let mut graph = self.load_project_graph()?;
        let id = self.upsert_memory_in_graph(&mut graph, entry);
        self.save_project_graph(&graph)?;
        Ok(id)
    }

    /// Insert or update a memory with a stable ID in the global graph.
    /// Preserves existing inbound/outbound graph relationships while refreshing
    /// content and tags.
    pub fn upsert_global_memory(&self, entry: MemoryEntry) -> Result<String> {
        crate::memory_quality::gate_entry(&entry)?;
        let mut graph = self.load_global_graph()?;
        let id = self.upsert_memory_in_graph(&mut graph, entry);
        self.save_global_graph(&graph)?;
        Ok(id)
    }

    fn upsert_memory_in_graph(
        &self,
        graph: &mut crate::memory_graph::MemoryGraph,
        entry: MemoryEntry,
    ) -> String {
        let id = entry.id.clone();

        let Some(existing_snapshot) = graph.get_memory(&id).cloned() else {
            return graph.add_memory(entry);
        };

        let old_tags: std::collections::HashSet<String> =
            existing_snapshot.tags.iter().cloned().collect();
        let new_tags: std::collections::HashSet<String> = entry.tags.iter().cloned().collect();

        for tag in old_tags.difference(&new_tags) {
            graph.untag_memory(&id, tag);
        }
        for tag in new_tags.difference(&old_tags) {
            graph.tag_memory(&id, tag);
        }

        if let Some(existing) = graph.get_memory_mut(&id) {
            existing.category = entry.category;
            existing.content = entry.content;
            existing.tags = entry.tags;
            existing.updated_at = entry.updated_at;
            existing.source = entry.source;
            existing.trust = entry.trust;
            existing.active = entry.active;
            existing.superseded_by = entry.superseded_by;
            existing.confidence = entry.confidence;
        }

        id
    }










    fn collect_memories_scoped(&self, scope: MemoryScope) -> Result<Vec<MemoryEntry>> {
        let mut entries = Vec::new();
        if scope.includes_project()
            && let Ok(project) = self.load_project_graph()
        {
            entries.extend(project.all_memories().cloned());
        }
        if scope.includes_global()
            && let Ok(global) = self.load_global_graph()
        {
            entries.extend(global.all_memories().cloned());
        }
        Ok(entries)
    }

    #[cfg(test)]
    fn synthetic_skill_entries(&self) -> Vec<MemoryEntry> {
        if !self.include_skills {
            return Vec::new();
        }

        collect_synthetic_entries()
    }

    #[cfg(test)]
    fn collect_retrieval_candidates_scoped(&self, scope: MemoryScope) -> Result<Vec<MemoryEntry>> {
        let mut entries = self.collect_memories_scoped(scope)?;
        if scope.includes_global() {
            entries.extend(self.synthetic_skill_entries());
        }
        Ok(entries)
    }




    pub fn get_prompt_memories(&self, limit: usize) -> Option<String> {
        self.get_prompt_memories_scoped(limit, MemoryScope::All)
    }

    pub fn get_prompt_memories_scoped(&self, limit: usize, scope: MemoryScope) -> Option<String> {
        let all_entries: Vec<_> = top_k_by_ord(
            self.collect_memories_scoped(scope)
                .ok()?
                .into_iter()
                .map(|entry| {
                    let updated_at = entry.updated_at.timestamp_millis();
                    (entry, updated_at)
                }),
            limit,
        )
        .into_iter()
        .map(|(entry, _)| entry)
        .collect();

        if all_entries.is_empty() {
            return None;
        }

        format_entries_for_prompt(&all_entries, limit)
    }

    pub fn search(&self, query: &str) -> Result<Vec<MemoryEntry>> {
        self.search_scoped(query, MemoryScope::All)
    }

    pub fn search_scoped(&self, query: &str, scope: MemoryScope) -> Result<Vec<MemoryEntry>> {
        let query_lower = normalize_search_text(query);
        if query_lower.is_empty() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();

        for memory in self.collect_memories_scoped(scope)? {
            if memory_matches_search(&memory, &query_lower) {
                results.push(memory);
            }
        }

        Ok(results)
    }

    pub fn list_all(&self) -> Result<Vec<MemoryEntry>> {
        self.list_all_scoped(MemoryScope::All)
    }

    pub fn list_all_scoped(&self, scope: MemoryScope) -> Result<Vec<MemoryEntry>> {
        let mut all = self.collect_memories_scoped(scope)?;
        all.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(all)
    }

    /// The database for a single-row edit, with the old JSON graphs imported first.
    fn store_db(&self) -> Result<PathBuf> {
        let db = self.db_path()?;
        if !self.test_mode {
            self.import_json_once(&db)?;
        }
        Ok(db)
    }

    /// `global` plus this manager's project scope: where its own `forget` and `tag` look.
    fn own_scopes(&self) -> Vec<String> {
        let mut scopes: Vec<String> = self.project_scope().into_iter().collect();
        scopes.push("global".to_string());
        scopes
    }

    /// Edit one memory in `scopes` (every scope when `None`) with a single-row write: never a
    /// whole-graph save, which would delete a concurrent writer's fresh rows. `Ok(false)` when the
    /// id is unknown; an error for a learned entry (managed through learning).
    pub(crate) fn edit_one(&self, id: &str, scopes: Option<&[String]>, edit: impl FnOnce(&mut MemoryGraph)) -> Result<bool> {
        let db = self.store_db()?;
        let scope = crate::memory_store::update_memory(&db, id, scopes, edit)?;
        if let Some(scope) = &scope {
            forget_graph(&format!("{}#{scope}", db.display()));
        }
        Ok(scope.is_some())
    }

    /// Fold duplicate `(scope, id)` into survivor `(scope, id)` (see `memory_store::merge_duplicate`).
    pub(crate) fn merge_duplicate(&self, survivor: (&str, &str), duplicate: (&str, &str)) -> Result<bool> {
        let db = self.store_db()?;
        let merged = crate::memory_store::merge_duplicate(&db, survivor, duplicate)?;
        for scope in [survivor.0, duplicate.0, "global"] {
            forget_graph(&format!("{}#{scope}", db.display()));
        }
        Ok(merged)
    }

    /// Forget `id` from this manager's project or global scope.
    pub fn forget(&self, id: &str) -> Result<bool> {
        self.forget_in(id, Some(&self.own_scopes()))
    }

    /// Forget `id` from whichever scope holds it (the maintenance screen acts on all scopes).
    pub fn forget_anywhere(&self, id: &str) -> Result<bool> {
        self.forget_in(id, None)
    }

    fn forget_in(&self, id: &str, scopes: Option<&[String]>) -> Result<bool> {
        let db = self.store_db()?;
        let scope = crate::memory_store::delete_memory(&db, id, scopes)?;
        if let Some(scope) = &scope {
            forget_graph(&format!("{}#{scope}", db.display()));
        }
        Ok(scope.is_some())
    }

    /// Replace one memory's text in any scope. `Ok(false)` when the id is unknown.
    pub fn edit_content(&self, id: &str, content: &str) -> Result<bool> {
        // An edit is a user action (desktop): high trust, content filters still apply.
        crate::memory_quality::gate_explicit(content, false, TrustLevel::High)?;
        self.edit_one(id, None, |graph| {
            if let Some(memory) = graph.get_memory_mut(id) {
                memory.content = content.to_string();
                memory.updated_at = chrono::Utc::now();
                memory.refresh_search_text();
            }
        })
    }

    /// Forget every non-learned memory in every scope: `only_preferences` `Some(true)` for
    /// preferences, `Some(false)` for everything else, `None` for all. Returns the ids forgotten.
    pub fn reset_all(&self, only_preferences: Option<bool>) -> Result<Vec<String>> {
        let db = self.store_db()?;
        let (deleted, scopes) = crate::memory_store::reset_memories(&db, only_preferences)?;
        for scope in scopes {
            forget_graph(&format!("{}#{scope}", db.display()));
        }
        Ok(deleted)
    }

    // === Async Memory Checking ===

    /// Local recall (factr): an indexed full-text query over the stored
    /// memories instead of loading and scanning all of them. Skips memories
    /// already injected into `session_id`, applies the never-pad floor
    /// (`memory_recall::meets_term_floor`) and returns at most `limit`, best first.
    pub fn recall_local(&self, session_id: Option<&str>, query: &str, limit: usize, scope: MemoryScope) -> Result<Vec<MemoryEntry>> {
        Ok(self.recall_local_detailed(session_id, query, limit, scope)?.entries)
    }

    /// [`Self::recall_local`] plus what it looked at: the query terms, how many candidates the
    /// index returned and how many were suppressed (already injected, or under the term floor).
    pub fn recall_local_detailed(&self, session_id: Option<&str>, query: &str, limit: usize, scope: MemoryScope) -> Result<LocalRecall> {
        let mut terms = crate::memory_recall::local_terms(query);
        terms.sort();
        terms.dedup();
        if terms.is_empty() || limit == 0 {
            return Ok(LocalRecall { terms, ..Default::default() });
        }
        let mut scopes = Vec::new();
        if scope.includes_global() {
            scopes.push("global".to_string());
        }
        if scope.includes_project()
            && let Some(project) = self.project_scope()
        {
            scopes.push(project);
        }
        let db = self.db_path()?;
        if !self.test_mode {
            self.import_json_once(&db)?;
        }
        // A few extra candidates so the injected/floor filters rarely starve the result.
        let candidates = crate::memory_store::search(&db, &scopes, &terms, limit * 4)?;
        let found = candidates.len();
        // BM25 order, re-weighted by trust, reinforcement and recency (nothing is hidden by this).
        let candidates = crate::memory_quality::rerank(candidates, chrono::Utc::now());
        let entries: Vec<MemoryEntry> = candidates
            .into_iter()
            .filter(|e| !learned::is_kept_out_of_recall(e))
            .filter(|e| session_id.is_none_or(|s| !is_memory_injected(s, &e.id)))
            .filter(|e| crate::memory_recall::meets_clause_floor(query, &terms, e))
            .take(limit)
            .collect();
        let suppressed = found.saturating_sub(entries.len());
        Ok(LocalRecall { terms, candidates: found, entries, suppressed })
    }

    /// Load the existing project graph.
    pub fn load_project_graph(&self) -> Result<MemoryGraph> {
        match self.project_scope() {
            Some(scope) => self.load_scope_graph(&scope, true),
            None => Ok(MemoryGraph::new()),
        }
    }

    /// Every stored scope's graph: `global` plus each `project:<hash>`, whatever
    /// this manager's own project is. For the maintenance screen, which acts on all of them.
    pub fn every_scope_graph(&self) -> Result<Vec<(String, MemoryGraph)>> {
        let db = self.db_path()?;
        if !self.test_mode {
            self.import_json_once(&db)?;
        }
        let mut scopes = crate::memory_store::scopes(&db)?;
        if !scopes.iter().any(|s| s == "global") {
            scopes.push("global".to_string());
        }
        scopes
            .into_iter()
            .map(|scope| Ok((scope.clone(), self.load_scope_graph(&scope, false)?)))
            .collect()
    }

    /// Save a graph loaded by [`Self::every_scope_graph`] back to its scope.
    pub fn save_graph_for_scope(&self, scope: &str, graph: &MemoryGraph) -> Result<()> {
        self.save_scope_graph(scope, graph)
    }

    /// Load global memories as a MemoryGraph
    pub fn load_global_graph(&self) -> Result<MemoryGraph> {
        self.load_scope_graph("global", false)
    }

    fn load_scope_graph(&self, scope: &str, import_notes: bool) -> Result<MemoryGraph> {
        let db = self.db_path()?;
        let key = format!("{}#{scope}", db.display());
        let version = crate::memory_store::data_version(&db)?;
        if !self.test_mode
            && let Some(graph) = cached_graph(&key, version)
        {
            return Ok(graph);
        }
        if !self.test_mode {
            self.import_json_once(&db)?;
        }
        let mut graph = crate::memory_store::load_graph(&db, scope)?.unwrap_or_default();
        let mut changed = Self::normalize_graph_search_text(&mut graph);
        if import_notes {
            changed |= self.import_legacy_notes_into_graph(&mut graph)?;
        }
        if changed {
            crate::memory_store::save_graph(&db, scope, &graph, None)?;
        }
        if !self.test_mode {
            cache_graph(key, crate::memory_store::data_version(&db)?, &graph);
        }
        Ok(graph)
    }

    /// A whole-graph save writes every row it is given, so it must not bring back one the gate
    /// rejects. A row that fails the gate and is already stored unchanged (written before the gate
    /// existed) is left for `memory audit`; a new or edited one fails the save.
    fn gate_graph_save(&self, db: &std::path::Path, scope: &str, graph: &MemoryGraph) -> Result<()> {
        let failing: Vec<&MemoryEntry> =
            graph.memories.values().filter(|e| crate::memory_quality::gate_entry(e).is_err()).collect();
        if failing.is_empty() {
            return Ok(());
        }
        let stored = crate::memory_store::load_graph(db, scope)?.unwrap_or_default();
        for entry in failing {
            if stored.memories.get(&entry.id).is_some_and(|s| s.content == entry.content) {
                continue;
            }
            crate::memory_quality::gate_entry(entry)?;
        }
        Ok(())
    }

    fn save_scope_graph(&self, scope: &str, graph: &MemoryGraph) -> Result<()> {
        let db = self.db_path()?;
        self.gate_graph_save(&db, scope, graph)?;
        if self.test_mode {
            return crate::memory_store::save_graph(&db, scope, graph, None);
        }
        let key = format!("{}#{scope}", db.display());
        let version = crate::memory_store::data_version(&db)?;
        // Diff against the graph as last loaded: only changed rows are written.
        with_cached(&key, version, |previous| crate::memory_store::save_graph(&db, scope, graph, previous))?;
        cache_graph(key, crate::memory_store::data_version(&db)?, graph);
        Ok(())
    }

    /// First use after the SQLite move: pull in the old JSON graphs once.
    fn import_json_once(&self, db: &std::path::Path) -> Result<()> {
        let memory_dir = storage::factr_dir()?.join("memory");
        let imported = crate::memory_store::import_json_once(db, &memory_dir, |path| {
            if let Ok(graph) = storage::read_json::<MemoryGraph>(path)
                && graph.graph_version == GRAPH_VERSION
            {
                return Ok(graph);
            }
            Ok(MemoryGraph::from_legacy_store(storage::read_json::<MemoryStore>(path)?))
        })?;
        if imported > 0 {
            crate::logging::info(&format!("Imported {imported} memories from JSON into {}", db.display()));
        }
        Ok(())
    }

    /// Save project memories as a MemoryGraph
    pub fn save_project_graph(&self, graph: &MemoryGraph) -> Result<()> {
        match self.project_scope() {
            Some(scope) => self.save_scope_graph(&scope, graph),
            None => Ok(()),
        }
    }

    /// Save global memories as a MemoryGraph
    pub fn save_global_graph(&self, graph: &MemoryGraph) -> Result<()> {
        self.save_scope_graph("global", graph)
    }

    /// Add a tag to a memory
    pub fn tag_memory(&self, memory_id: &str, tag: &str) -> Result<()> {
        if self.edit_one(memory_id, Some(&self.own_scopes()), |graph| graph.tag_memory(memory_id, tag))? {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Memory not found: {}", memory_id))
        }
    }

    /// Link two memories with a RelatesTo edge
    pub fn link_memories(&self, from_id: &str, to_id: &str, weight: f32) -> Result<()> {
        // Try project first
        let mut graph = self.load_project_graph()?;
        if graph.memories.contains_key(from_id) && graph.memories.contains_key(to_id) {
            graph.link_memories(from_id, to_id, weight);
            return self.save_project_graph(&graph);
        }

        // Try global
        let mut graph = self.load_global_graph()?;
        if graph.memories.contains_key(from_id) && graph.memories.contains_key(to_id) {
            graph.link_memories(from_id, to_id, weight);
            return self.save_global_graph(&graph);
        }

        // Cross-store links not supported for now
        Err(anyhow::anyhow!(
            "Both memories must be in the same store (project or global)"
        ))
    }

    /// Get memories related to a given memory via graph traversal
    pub fn get_related(&self, memory_id: &str, depth: usize) -> Result<Vec<MemoryEntry>> {
        // Find which store contains the memory
        let (mut graph, _is_project) = {
            let project_graph = self.load_project_graph()?;
            if project_graph.memories.contains_key(memory_id) {
                (project_graph, true)
            } else {
                let global_graph = self.load_global_graph()?;
                if global_graph.memories.contains_key(memory_id) {
                    (global_graph, false)
                } else {
                    return Err(anyhow::anyhow!("Memory not found: {}", memory_id));
                }
            }
        };

        // Use cascade retrieval to find related memories
        let results = graph.cascade_retrieve(&[memory_id.to_string()], &[1.0], depth, 20);

        // Collect memory entries (excluding the seed)
        let entries: Vec<MemoryEntry> = results
            .into_iter()
            .filter(|(id, _)| id != memory_id)
            .filter_map(|(id, _)| graph.get_memory(&id).cloned())
            .collect();

        Ok(entries)
    }



    /// Get graph statistics for display
    pub fn graph_stats(&self) -> Result<(usize, usize, usize, usize)> {
        let project = self.load_project_graph()?;
        let global = self.load_global_graph()?;

        let memories = project.memories.len() + global.memories.len();
        let tags = project.tags.len() + global.tags.len();
        let edges = project.edge_count() + global.edge_count();
        let clusters = project.clusters.len() + global.clusters.len();

        Ok((memories, tags, edges, clusters))
    }
}


impl Default for MemoryManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Drop `extracted_through:<session>` from the database at `db` (a forgotten session's extraction
/// bookkeeping). Meant for the code that deletes a session's learned entries
/// (`factr-learn/src/entries.rs` `forget_session`); not yet called from there.
pub fn forget_session_meta(db: &std::path::Path, session: &str) -> Result<usize> {
    crate::memory_store::forget_session_meta(db, session)
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;

#[cfg(test)]
mod scope_hash_tests {
    /// Project scopes are `DefaultHasher` of the directory, and the directory itself is not stored,
    /// so a toolchain that changes the hash would orphan every project scope with no way to migrate.
    #[test]
    fn project_scope_hash_is_pinned() {
        assert_eq!(super::learned::project_scope("/work/a"), "project:8c9f0d145e24a070");
    }
}

#[cfg(test)]
mod single_row_tests {
    use super::*;

    #[test]
    fn forget_expire_and_tag_do_not_delete_a_row_remembered_meanwhile() {
        let _guard = crate::storage::lock_test_env();
        let old = std::env::var("FACTR_HOME").ok();
        let dir = std::env::temp_dir().join(format!("factr-single-row-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        crate::env::set_var("FACTR_HOME", &dir);
        let manager = MemoryManager::new_test();
        let a = manager.remember_global(MemoryEntry::new(MemoryCategory::Fact, "alpha deploy rule")).unwrap();
        let b = manager.remember_global(MemoryEntry::new(MemoryCategory::Fact, "beta cache rule")).unwrap();
        let stale = manager.load_global_graph().unwrap();
        // The extractor remembers while a maintenance call is between its load and its write.
        let fresh = manager.remember_global(MemoryEntry::new(MemoryCategory::Fact, "gamma fresh extraction")).unwrap();
        assert!(manager.expire(&a, "wrong").unwrap());
        assert!(manager.forget(&b).unwrap());
        manager.tag_memory(&fresh, "extracted").unwrap();
        let after = manager.load_global_graph().unwrap();
        assert!(after.memories.contains_key(&fresh), "the fresh memory survives expire, forget and tag");
        assert!(after.memories[&fresh].tags.contains(&"extracted".to_string()));
        assert!(!after.memories[&a].active && !after.memories.contains_key(&b));
        assert_eq!(stale.memories.len(), 2);
        assert!(manager.tag_memory("nope", "x").is_err());
        match old {
            Some(v) => crate::env::set_var("FACTR_HOME", v),
            None => crate::env::remove_var("FACTR_HOME"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
