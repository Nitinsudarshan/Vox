//! A cached, incrementally-updated index over the scribbles directory.
//!
//! Every knowledge-graph read used to go through `list_scribbles()`, which
//! `read_dir`s the scribbles directory and re-parses every Markdown file in
//! it — JSON frontmatter and all — on every single call. Opening the graph,
//! switching a filter and saving a thought each paid the full cost of the
//! vault. Nothing about that scales, and every view built on the graph
//! inherits it.
//!
//! This index parses a file once and keeps the result, keyed by path and
//! stamped with the file's modification time and size. A refresh stats each
//! file and re-reads only those whose stamp moved. The built graph —
//! including PageRank, which is the expensive part — is cached alongside and
//! invalidated only when a file actually changed.
//!
//! ## What the stamp can and cannot catch
//!
//! `mtime` + size misses a write that lands in the same filesystem mtime
//! tick *and* leaves the byte count identical. On a filesystem with
//! one-second mtime granularity that is reachable: correcting a typo twice
//! in the same second. The vault's own writers are not the risk (each
//! `save_scribble` bumps `updated_at`, which changes the length of the
//! frontmatter); an external editor is. `invalidate()` exists for callers
//! that know they have written, and is what the vault's own write paths
//! call, so the stamp is a fast path for *foreign* edits rather than the
//! only line of defence.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

use thiserror::Error;

use crate::sync::MutexExt;

use super::scribble::{GraphFilter, KnowledgeGraphData, Scribble};

#[derive(Error, Debug)]
pub enum ScribbleIndexError {
    #[error("Could not list the scribbles directory {dir:?}: {source}")]
    ListDir {
        dir: PathBuf,
        source: std::io::Error,
    },

    #[error("Could not read the scribbles directory entry in {dir:?}: {source}")]
    ReadEntry {
        dir: PathBuf,
        source: std::io::Error,
    },
}

/// What a refresh did, for logging and for the tests that assert
/// incrementality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RefreshStats {
    /// Scribble files present in the directory afterwards.
    pub total: usize,
    /// Files read from disk and parsed during this refresh.
    pub reparsed: usize,
    /// Cached entries dropped because their file is gone.
    pub removed: usize,
    /// Files skipped because their stamp was unchanged.
    pub reused: usize,
}

impl RefreshStats {
    /// Whether this refresh changed the set of parsed scribbles at all.
    fn changed(&self) -> bool {
        self.reparsed > 0 || self.removed > 0
    }
}

/// The identity of a file's contents, as cheaply as the filesystem will say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    /// `None` where the platform cannot report one, which forces a re-parse
    /// every time rather than trusting size alone.
    modified: Option<SystemTime>,
    size: u64,
}

impl FileStamp {
    fn matches(&self, other: &FileStamp) -> bool {
        match (self.modified, other.modified) {
            (Some(a), Some(b)) => a == b && self.size == other.size,
            _ => false,
        }
    }
}

struct IndexedFile {
    stamp: FileStamp,
    scribble: Scribble,
}

#[derive(Default)]
struct IndexState {
    /// The directory these entries came from. The vault root is repointable
    /// at runtime, and entries from the old root describe a different vault.
    dir: Option<PathBuf>,
    files: HashMap<PathBuf, IndexedFile>,
    /// The unfiltered graph over `files`, built on demand and dropped
    /// whenever the file set changes.
    graph: Option<KnowledgeGraphData>,
}

/// A parse cache over the scribbles directory.
///
/// Held by `VaultManager` and shared across threads. The lock is held across
/// the file reads a refresh performs, which serialises two concurrent
/// refreshes rather than letting both re-parse the same vault — the work is
/// the thing worth avoiding, not the wait.
pub struct ScribbleIndex {
    state: Mutex<IndexState>,
    /// Files whose contents were read from disk, over the index's lifetime.
    /// The number the cache exists to hold down, so it is observable.
    files_read: AtomicU64,
    /// Frontmatter documents parsed. Tracks `files_read` except where a file
    /// is read but fails to parse.
    parses: AtomicU64,
}

impl Default for ScribbleIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl ScribbleIndex {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(IndexState::default()),
            files_read: AtomicU64::new(0),
            parses: AtomicU64::new(0),
        }
    }

    /// How many scribble files have been read from disk since startup.
    pub fn files_read(&self) -> u64 {
        self.files_read.load(Ordering::Relaxed)
    }

    /// How many frontmatter documents have been parsed since startup.
    pub fn parses(&self) -> u64 {
        self.parses.load(Ordering::Relaxed)
    }

    /// Drops everything, forcing the next read to rebuild from disk.
    ///
    /// The vault's write paths call this: a writer knows it changed a file,
    /// and knowing beats inferring from a timestamp whose granularity it
    /// does not control.
    pub fn invalidate(&self) {
        let mut state = self.state.lock_or_recover();
        state.files.clear();
        state.graph = None;
    }

    /// Every active scribble, newest first, with dangling relationships
    /// removed.
    ///
    /// Matches what `list_scribbles` returned before the index existed,
    /// including the cleanup pass: a relationship pointing at a merged,
    /// trashed or deleted scribble is not a link the caller should see.
    /// That pass runs per call rather than at parse time because which
    /// targets exist is a property of the directory, not of the file.
    pub fn scribbles(&self, dir: &Path) -> Result<Vec<Scribble>, ScribbleIndexError> {
        let mut state = self.state.lock_or_recover();
        self.refresh_locked(&mut state, dir)?;
        Ok(Self::collect_scribbles(&state))
    }

    /// The knowledge graph, filtered on the way out.
    ///
    /// The unfiltered graph is built once per vault change and reused;
    /// `filter` only ever removes from it. Two calls with different filters
    /// therefore read no files and rebuild nothing, which is what lets the
    /// graph surface switch view modes and toggle filters without a refetch.
    pub fn graph(
        &self,
        dir: &Path,
        filter: Option<&GraphFilter>,
    ) -> Result<KnowledgeGraphData, ScribbleIndexError> {
        let mut state = self.state.lock_or_recover();
        self.refresh_locked(&mut state, dir)?;

        if state.graph.is_none() {
            let span = tracing::info_span!("scribble_index.build_graph");
            let _guard = span.enter();
            let scribbles = Self::collect_scribbles(&state);
            let built = KnowledgeGraphData::build(&scribbles);
            tracing::debug!(
                nodes = built.nodes.len(),
                edges = built.edges.len(),
                "rebuilt the knowledge graph from the scribble index"
            );
            state.graph = Some(built);
        }

        // `graph` was just populated above if it was empty.
        let Some(cached) = state.graph.as_ref() else {
            return Ok(KnowledgeGraphData::default());
        };
        Ok(match filter {
            Some(f) => cached.filtered(f),
            None => cached.clone(),
        })
    }

    /// Brings the cache in line with what is on disk.
    fn refresh_locked(
        &self,
        state: &mut IndexState,
        dir: &Path,
    ) -> Result<RefreshStats, ScribbleIndexError> {
        let span = tracing::info_span!("scribble_index.refresh", dir = ?dir);
        let _guard = span.enter();

        if state.dir.as_deref() != Some(dir) {
            tracing::debug!(previous = ?state.dir, "scribble index repointed at a new vault");
            state.files.clear();
            state.graph = None;
            state.dir = Some(dir.to_path_buf());
        }

        if !dir.exists() {
            let removed = state.files.len();
            state.files.clear();
            if removed > 0 {
                state.graph = None;
            }
            return Ok(RefreshStats {
                removed,
                ..Default::default()
            });
        }

        let entries = std::fs::read_dir(dir).map_err(|source| ScribbleIndexError::ListDir {
            dir: dir.to_path_buf(),
            source,
        })?;

        let mut stats = RefreshStats::default();
        let mut seen: HashSet<PathBuf> = HashSet::new();

        for entry in entries {
            let entry = entry.map_err(|source| ScribbleIndexError::ReadEntry {
                dir: dir.to_path_buf(),
                source,
            })?;
            let path = entry.path();
            if !path.extension().is_some_and(|ext| ext == "md") {
                continue;
            }

            // A file that cannot be stat'd is skipped rather than fatal: one
            // unreadable entry must not empty the graph.
            let Ok(metadata) = entry.metadata() else {
                tracing::warn!(?path, "could not stat a scribble file; skipping it");
                continue;
            };
            let stamp = FileStamp {
                modified: metadata.modified().ok(),
                size: metadata.len(),
            };

            seen.insert(path.clone());
            stats.total += 1;

            if state
                .files
                .get(&path)
                .is_some_and(|cached| cached.stamp.matches(&stamp))
            {
                stats.reused += 1;
                continue;
            }

            self.files_read.fetch_add(1, Ordering::Relaxed);
            let Ok(content) = std::fs::read_to_string(&path) else {
                tracing::warn!(?path, "could not read a scribble file; skipping it");
                state.files.remove(&path);
                stats.total -= 1;
                seen.remove(&path);
                continue;
            };

            self.parses.fetch_add(1, Ordering::Relaxed);
            match Scribble::parse_markdown(&content) {
                Some(scribble) => {
                    state.files.insert(path, IndexedFile { stamp, scribble });
                    stats.reparsed += 1;
                }
                None => {
                    // Unparseable frontmatter. Drop any stale entry so the
                    // graph stops showing a version of the file that is no
                    // longer what is on disk.
                    tracing::warn!(?path, "scribble frontmatter did not parse; excluding it");
                    if state.files.remove(&path).is_some() {
                        stats.removed += 1;
                    }
                    stats.total -= 1;
                    seen.remove(&path);
                }
            }
        }

        let gone: Vec<PathBuf> = state
            .files
            .keys()
            .filter(|p| !seen.contains(*p))
            .cloned()
            .collect();
        stats.removed += gone.len();
        for path in gone {
            state.files.remove(&path);
        }

        if stats.changed() {
            state.graph = None;
        }

        tracing::debug!(
            total = stats.total,
            reparsed = stats.reparsed,
            reused = stats.reused,
            removed = stats.removed,
            "scribble index refreshed"
        );
        Ok(stats)
    }

    /// The cached scribbles as the vault presents them: dangling
    /// relationships dropped, newest first.
    fn collect_scribbles(state: &IndexState) -> Vec<Scribble> {
        let mut scribbles: Vec<Scribble> =
            state.files.values().map(|f| f.scribble.clone()).collect();

        let valid_ids: HashSet<String> = scribbles.iter().map(|s| s.id.clone()).collect();
        for s in &mut scribbles {
            s.relationships.retain(|r| valid_ids.contains(&r.target_id));
        }

        // Newest first, with the id as the tiebreak. Two scribbles saved in
        // the same millisecond would otherwise order by `HashMap` iteration,
        // which differs between runs and would make the graph's node order —
        // and so the layout seeded from it — irreproducible.
        scribbles.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        scribbles
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::scribble::{GraphFilter, ScribbleRelationship, REL_RELATED_TO};
    use std::time::{Duration, Instant};

    /// A scribbles directory that cleans itself up.
    struct TempScribbles {
        dir: PathBuf,
    }

    impl TempScribbles {
        fn new() -> Self {
            let dir =
                std::env::temp_dir().join(format!("vox_scribble_index_{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).expect("create the temp scribbles directory");
            Self { dir }
        }

        fn write(&self, scribble: &Scribble) -> PathBuf {
            let path = self.dir.join(format!("{}.md", scribble.id));
            std::fs::write(&path, scribble.format_markdown()).expect("write a scribble");
            path
        }

        /// Writes `count` scribbles, each with a topic shared with its
        /// neighbour so the graph has edges to rank.
        fn seed(&self, count: usize) -> Vec<Scribble> {
            let mut written = Vec::with_capacity(count);
            for i in 0..count {
                let mut s = Scribble::new_text(
                    &format!("Body of thought number {i}, long enough to be worth parsing."),
                    Some(&format!("Thought {i}")),
                );
                s.topics = vec![format!("topic-{}", i % 7)];
                s.entities = vec![format!("entity-{}", i % 11)];
                self.write(&s);
                written.push(s);
            }
            written
        }
    }

    impl Drop for TempScribbles {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Moves a file's modification time forward without changing a byte of
    /// it — a genuine `touch`, so the test exercises the mtime half of the
    /// stamp rather than the size half.
    fn touch(path: &Path) {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open for touch");
        let later = std::fs::metadata(path)
            .expect("stat before touch")
            .modified()
            .expect("mtime is available on this platform")
            + Duration::from_secs(30);
        file.set_modified(later).expect("set mtime");
    }

    /// The headline claim: reading the graph a second time re-reads nothing.
    ///
    /// Fails if `files_read` increments on the second call — that is the
    /// unindexed behaviour this module replaced.
    #[test]
    fn a_second_graph_read_reads_no_files() {
        let vault = TempScribbles::new();
        vault.seed(25);
        let index = ScribbleIndex::new();

        let first = index.graph(&vault.dir, None).expect("first graph read");
        let reads_after_cold = index.files_read();
        assert_eq!(
            reads_after_cold, 25,
            "the cold read should read every scribble exactly once"
        );

        let second = index.graph(&vault.dir, None).expect("second graph read");
        assert_eq!(
            index.files_read(),
            reads_after_cold,
            "the second read must not touch the filesystem"
        );
        assert_eq!(
            index.parses(),
            25,
            "nothing should be re-parsed for an unchanged vault"
        );
        assert_eq!(first, second, "a cached graph must equal the built one");
    }

    /// Fails at 0 (the index served a stale file) or at 25 (the index threw
    /// the cache away and rebuilt everything).
    #[test]
    fn touching_one_file_reparses_exactly_that_file() {
        let vault = TempScribbles::new();
        let written = vault.seed(25);
        let index = ScribbleIndex::new();

        index.graph(&vault.dir, None).expect("cold read");
        let baseline = index.parses();
        assert_eq!(baseline, 25);

        touch(&vault.dir.join(format!("{}.md", written[7].id)));

        index.graph(&vault.dir, None).expect("incremental read");
        assert_eq!(
            index.parses() - baseline,
            1,
            "exactly the touched file should have been re-parsed"
        );
        assert_eq!(
            index.files_read() - 25,
            1,
            "exactly the touched file should have been re-read"
        );
    }

    /// An edit must actually reach the graph — an index that never
    /// invalidates would pass the two counter tests above and still be wrong.
    #[test]
    fn an_edited_file_changes_what_the_graph_says() {
        let vault = TempScribbles::new();
        let mut written = vault.seed(4);
        let index = ScribbleIndex::new();

        let before = index.graph(&vault.dir, None).expect("cold read");
        assert!(before.nodes.iter().any(|n| n.label == "Thought 2"));

        written[2].title = "Renamed thought".to_string();
        written[2].updated_at = chrono::Utc::now().to_rfc3339();
        vault.write(&written[2]);

        let after = index.graph(&vault.dir, None).expect("read after the edit");
        assert!(
            after.nodes.iter().any(|n| n.label == "Renamed thought"),
            "the edit should be visible in the graph"
        );
        assert!(
            !after.nodes.iter().any(|n| n.label == "Thought 2"),
            "the pre-edit node should be gone, not duplicated"
        );
    }

    #[test]
    fn a_deleted_file_leaves_the_index_and_the_graph() {
        let vault = TempScribbles::new();
        let written = vault.seed(5);
        let index = ScribbleIndex::new();

        assert_eq!(index.scribbles(&vault.dir).expect("cold read").len(), 5);
        std::fs::remove_file(vault.dir.join(format!("{}.md", written[1].id))).expect("delete");

        let remaining = index.scribbles(&vault.dir).expect("read after delete");
        assert_eq!(remaining.len(), 4);
        assert!(!remaining.iter().any(|s| s.id == written[1].id));
        assert_eq!(
            index.files_read(),
            5,
            "deleting a file should not re-read the survivors"
        );
    }

    /// Filtering is what the view modes do constantly. It must cost nothing
    /// on disk and must not disturb what survives it.
    #[test]
    fn changing_the_filter_reads_nothing_and_moves_no_node() {
        let vault = TempScribbles::new();
        vault.seed(12);
        let index = ScribbleIndex::new();

        let unfiltered = index.graph(&vault.dir, None).expect("cold read");
        let reads = index.files_read();

        let filter = GraphFilter {
            include_topics: Some(false),
            include_entities: Some(false),
            ..Default::default()
        };
        let filtered = index
            .graph(&vault.dir, Some(&filter))
            .expect("filtered read");

        assert_eq!(index.files_read(), reads, "filtering must not read files");
        assert!(filtered.nodes.iter().all(|n| n.node_type == "scribble"));

        // A surviving node is the same node, PageRank included.
        for node in &filtered.nodes {
            let original = unfiltered
                .nodes
                .iter()
                .find(|n| n.id == node.id)
                .expect("a filtered node must exist in the unfiltered graph");
            assert_eq!(original, node, "filtering must not rewrite a node");
        }
    }

    /// Repointing the vault must not serve the previous vault's contents.
    #[test]
    fn repointing_at_another_directory_starts_over() {
        let first = TempScribbles::new();
        let second = TempScribbles::new();
        first.seed(3);
        second.seed(6);
        let index = ScribbleIndex::new();

        assert_eq!(index.scribbles(&first.dir).expect("first vault").len(), 3);
        assert_eq!(index.scribbles(&second.dir).expect("second vault").len(), 6);
        assert_eq!(index.scribbles(&first.dir).expect("back again").len(), 3);
    }

    #[test]
    fn a_missing_directory_is_an_empty_graph_rather_than_an_error() {
        let index = ScribbleIndex::new();
        let missing = std::env::temp_dir().join(format!("vox_absent_{}", uuid::Uuid::new_v4()));

        let graph = index
            .graph(&missing, None)
            .expect("a missing vault is empty");
        assert!(graph.nodes.is_empty());
        assert_eq!(index.files_read(), 0);
    }

    /// Unparseable frontmatter costs its own file and nothing else.
    #[test]
    fn a_corrupt_file_is_skipped_without_emptying_the_graph() {
        let vault = TempScribbles::new();
        vault.seed(3);
        std::fs::write(
            vault.dir.join("broken.md"),
            "---\nnot json at all\n---\nbody",
        )
        .expect("write a corrupt scribble");

        let index = ScribbleIndex::new();
        let scribbles = index
            .scribbles(&vault.dir)
            .expect("read past the corruption");
        assert_eq!(scribbles.len(), 3);
    }

    /// Relationship cleanup survived the move into the index: a link to a
    /// scribble that is no longer in the vault is not a link to show.
    #[test]
    fn dangling_relationships_are_dropped_per_read() {
        let vault = TempScribbles::new();
        let written = vault.seed(2);
        let mut linker = Scribble::new_text("Links to a thought that leaves", Some("Linker"));
        linker.relationships.push(ScribbleRelationship {
            id: "rel_1".to_string(),
            target_id: written[0].id.clone(),
            relationship_type: REL_RELATED_TO.to_string(),
            confidence: 1.0,
            source: "user".to_string(),
        });
        vault.write(&linker);

        let index = ScribbleIndex::new();
        let before = index.scribbles(&vault.dir).expect("cold read");
        let found = before.iter().find(|s| s.id == linker.id).expect("linker");
        assert_eq!(found.relationships.len(), 1);

        std::fs::remove_file(vault.dir.join(format!("{}.md", written[0].id))).expect("delete");

        let after = index.scribbles(&vault.dir).expect("read after delete");
        let found = after.iter().find(|s| s.id == linker.id).expect("linker");
        assert!(
            found.relationships.is_empty(),
            "the link should be dropped now its target is gone, \
             even though the linker itself was never re-parsed"
        );
    }

    /// Node order must not depend on `HashMap` iteration, or the layout
    /// seeded from it changes between runs of the same vault.
    #[test]
    fn the_same_vault_produces_the_same_graph_twice_over() {
        let vault = TempScribbles::new();
        vault.seed(30);

        let a = ScribbleIndex::new()
            .graph(&vault.dir, None)
            .expect("first index");
        let b = ScribbleIndex::new()
            .graph(&vault.dir, None)
            .expect("second, independent index");

        assert_eq!(a.nodes.len(), b.nodes.len());
        for (left, right) in a.nodes.iter().zip(b.nodes.iter()) {
            assert_eq!(left.id, right.id, "node order must be reproducible");
            assert_eq!(
                left.pagerank.to_bits(),
                right.pagerank.to_bits(),
                "PageRank must be bit-identical across runs"
            );
        }
    }

    /// Reports the cold-index cost. Run with `--nocapture` to read it.
    ///
    /// The assertion is a ceiling loose enough to only catch a regression of
    /// a different order — the number itself is the point, not the bound.
    #[test]
    fn reports_cold_index_time_for_a_realistic_vault() {
        const N: usize = 500;
        let vault = TempScribbles::new();
        vault.seed(N);

        let index = ScribbleIndex::new();
        let started = Instant::now();
        let graph = index.graph(&vault.dir, None).expect("cold index");
        let cold = started.elapsed();

        let started = Instant::now();
        index.graph(&vault.dir, None).expect("warm index");
        let warm = started.elapsed();

        println!(
            "cold index of {N} scribbles: {cold:?} ({} nodes, {} edges); warm re-read: {warm:?}",
            graph.nodes.len(),
            graph.edges.len()
        );

        assert_eq!(index.files_read(), N as u64);
        assert!(
            cold < Duration::from_secs(30),
            "indexing {N} scribbles took {cold:?}, which is a different order of cost"
        );
    }
}
