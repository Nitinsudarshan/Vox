use super::para::{deserialize_band_lenient, ParaBand};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Supported capture source types for Scribbles.
pub const SOURCE_TYPE_VOICE: &str = "voice";
pub const SOURCE_TYPE_TEXT: &str = "text";
pub const SOURCE_TYPE_FILE: &str = "file";
pub const SOURCE_TYPE_MEETING: &str = "meeting";
pub const SOURCE_TYPE_CLIPBOARD: &str = "clipboard";
pub const SOURCE_TYPE_BROWSER_SELECTION: &str = "browser_selection";
pub const SOURCE_TYPE_BROWSER_PAGE: &str = "browser_page";
pub const SOURCE_TYPE_BROWSER_CONVERSATION: &str = "browser_conversation";
pub const SOURCE_TYPE_SCREENSHOT: &str = "screenshot";
pub const SOURCE_TYPE_IMAGE: &str = "image";

/// Relationship types between knowledge objects.
pub const REL_RELATED_TO: &str = "RELATED_TO";
pub const REL_MENTIONS: &str = "MENTIONS";
pub const REL_SAME_TOPIC: &str = "SAME_TOPIC";
pub const REL_SAME_PROJECT: &str = "SAME_PROJECT";
pub const REL_CONTRADICTS: &str = "CONTRADICTS";
pub const REL_EXTENDS: &str = "EXTENDS";
pub const REL_DERIVED_FROM: &str = "DERIVED_FROM";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScribbleRelationship {
    pub id: String,
    pub target_id: String,
    pub relationship_type: String,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    #[serde(default = "default_source")]
    pub source: String, // "ai" | "user" | "system"
}

fn default_confidence() -> f32 {
    1.0
}

fn default_source() -> String {
    "user".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ScribbleAttachment {
    pub id: String,
    pub filename: String,
    pub path: String,
    pub mime_type: Option<String>,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ScribbleAiMetadata {
    #[serde(default = "default_enrichment_status")]
    pub enrichment_status: String, // "pending" | "enriched" | "failed" | "none"
    #[serde(default)]
    pub suggested_concepts: Vec<String>,
    #[serde(default)]
    pub suggested_questions: Vec<String>,
    #[serde(default)]
    pub suggested_relations: Vec<String>,
    #[serde(default)]
    pub last_enriched_at: Option<String>,
}

fn default_enrichment_status() -> String {
    "none".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Scribble {
    pub id: String,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default = "default_source_type")]
    pub source_type: String,
    #[serde(default)]
    pub source_metadata: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub entities: Vec<String>,
    /// Which PARA band this thought is filed under, if any.
    ///
    /// `None` means uncategorised — a real state, shown as its own region,
    /// not a band to be guessed at from topics or content.
    #[serde(default, deserialize_with = "deserialize_band_lenient")]
    pub para: Option<ParaBand>,
    #[serde(default)]
    pub relationships: Vec<ScribbleRelationship>,
    #[serde(default)]
    pub attachments: Vec<ScribbleAttachment>,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub ai_metadata: ScribbleAiMetadata,
}

fn default_source_type() -> String {
    SOURCE_TYPE_TEXT.to_string()
}

fn default_status() -> String {
    "active".to_string()
}

impl Scribble {
    pub fn new_text(content: &str, title: Option<&str>) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        let derived_title = match title {
            Some(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => {
                if content.trim().is_empty() {
                    "Untitled Thought".to_string()
                } else {
                    "Generating title…".to_string()
                }
            }
        };

        Self {
            id: format!("scribble_{}", uuid::Uuid::new_v4()),
            title: derived_title,
            content: content.to_string(),
            summary: None,
            source_type: SOURCE_TYPE_TEXT.to_string(),
            source_metadata: serde_json::json!({}),
            created_at: now.clone(),
            updated_at: now,
            tags: Vec::new(),
            topics: Vec::new(),
            entities: Vec::new(),
            para: None,
            relationships: Vec::new(),
            attachments: Vec::new(),
            status: "active".to_string(),
            ai_metadata: ScribbleAiMetadata {
                enrichment_status: "pending".to_string(),
                ..Default::default()
            },
        }
    }

    pub fn from_voice_note(
        voice_note_id: &str,
        transcript: &str,
        custom_title: Option<&str>,
    ) -> Self {
        let initial_title = custom_title
            .map(|t| t.to_string())
            .unwrap_or_else(|| crate::pipeline::extract_deterministic_title(transcript));
        let mut scribble = Self::new_text(transcript, Some(&initial_title));
        scribble.source_type = SOURCE_TYPE_VOICE.to_string();
        scribble.source_metadata = serde_json::json!({
            "source_voice_note_id": voice_note_id,
            "source_voice_note_ids": vec![voice_note_id],
            "is_merged": false,
            "source_modality": "VOICE",
            "promoted_at": chrono::Utc::now().to_rfc3339()
        });
        scribble
    }


    /// A Scribble promoted from a recorded meeting.
    ///
    /// `body` is the generated report where one exists and the rendered
    /// transcript otherwise — a meeting with no summary is still a record
    /// worth connecting into the graph. The meeting id is kept in
    /// `source_metadata` so the Scribble can be traced back to its recording
    /// and its audio.
    pub fn from_meeting(meeting_id: &str, title: &str, body: &str) -> Self {
        let mut scribble = Self::new_text(body, Some(title));
        scribble.source_type = SOURCE_TYPE_MEETING.to_string();
        scribble.source_metadata = serde_json::json!({
            "source_meeting_id": meeting_id,
            "source_modality": "MEETING",
            "promoted_at": chrono::Utc::now().to_rfc3339()
        });
        scribble
    }

    pub fn from_file(
        filename: &str,
        content: &str,
        mime_type: Option<&str>,
        size_bytes: Option<u64>,
    ) -> Self {
        let mut scribble = Self::new_text(content, Some(filename));
        scribble.source_type = SOURCE_TYPE_FILE.to_string();
        scribble.source_metadata = serde_json::json!({
            "filename": filename,
            "mime_type": mime_type,
            "size_bytes": size_bytes,
            "imported_at": chrono::Utc::now().to_rfc3339()
        });
        scribble
    }

    pub fn format_markdown(&self) -> String {
        let frontmatter_struct = ScribbleFrontmatter {
            id: self.id.clone(),
            title: self.title.clone(),
            source_type: self.source_type.clone(),
            source_metadata: self.source_metadata.clone(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            tags: self.tags.clone(),
            topics: self.topics.clone(),
            entities: self.entities.clone(),
            para: self.para,
            relationships: self.relationships.clone(),
            attachments: self.attachments.clone(),
            status: self.status.clone(),
            summary: self.summary.clone(),
            ai_metadata: self.ai_metadata.clone(),
        };

        let json_meta = serde_json::to_string_pretty(&frontmatter_struct)
            .unwrap_or_else(|_| "{}".to_string());

        format!("---\n{}\n---\n\n{}", json_meta, self.content)
    }

    pub fn parse_markdown(raw: &str) -> Option<Self> {
        let parts: Vec<&str> = raw.splitn(3, "---").collect();
        if parts.len() < 3 {
            return None;
        }

        let frontmatter_str = parts[1].trim();
        let body = parts[2].trim_start_matches('\n').to_string();

        let meta: ScribbleFrontmatter = serde_json::from_str(frontmatter_str).ok()?;

        Some(Scribble {
            id: meta.id,
            title: meta.title,
            content: body,
            summary: meta.summary,
            source_type: meta.source_type,
            source_metadata: meta.source_metadata,
            created_at: meta.created_at,
            updated_at: meta.updated_at,
            tags: meta.tags,
            topics: meta.topics,
            entities: meta.entities,
            para: meta.para,
            relationships: meta.relationships,
            attachments: meta.attachments,
            status: meta.status,
            ai_metadata: meta.ai_metadata,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScribbleFrontmatter {
    pub id: String,
    pub title: String,
    #[serde(default = "default_source_type")]
    pub source_type: String,
    #[serde(default)]
    pub source_metadata: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub entities: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_band_lenient")]
    pub para: Option<ParaBand>,
    #[serde(default)]
    pub relationships: Vec<ScribbleRelationship>,
    #[serde(default)]
    pub attachments: Vec<ScribbleAttachment>,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub ai_metadata: ScribbleAiMetadata,
}

/// Knowledge Graph Node representation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgeNode {
    pub id: String,
    pub node_type: String, // "scribble" | "topic" | "entity" | "source" | "project" | "document" | "task" | "voice_note"
    pub label: String,
    pub summary: Option<String>,
    pub metadata: serde_json::Value,
    pub degree: usize,
    pub source_type: Option<String>,
    /// Structural importance, computed once over the whole graph.
    ///
    /// Node size in the Rings view reads this rather than `degree`: degree
    /// counts a node's own links, PageRank counts whether those links come
    /// from anywhere that matters. Computed before filtering, so hiding
    /// topics does not resize what is left.
    #[serde(default)]
    pub pagerank: f32,
    /// The PARA band this node sits in, if it has one.
    ///
    /// Only scribbles carry a band: it comes from the scribble's own
    /// frontmatter. Topics, entities and source records are not filed under
    /// PARA at all, so they stay `None` and render in the uncategorised
    /// region rather than being assigned a band on the strength of what they
    /// happen to connect to.
    #[serde(default)]
    pub para: Option<ParaBand>,
    /// When the underlying object last changed, where that is known.
    ///
    /// Drives the Rings view's recency temperature. Derived nodes (topics,
    /// entities) take the most recent `updated_at` of the scribbles that
    /// produced them — a topic is exactly as fresh as the last thought filed
    /// under it.
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Knowledge Graph Edge representation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnowledgeEdge {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub relationship: String,
    pub confidence: f32,
    pub source: String, // "ai" | "user" | "system"
}

/// Full Knowledge Graph payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct KnowledgeGraphData {
    pub nodes: Vec<KnowledgeNode>,
    pub edges: Vec<KnowledgeEdge>,
}

/// Filter options for graph rendering and analysis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct GraphFilter {
    pub include_scribbles: Option<bool>,
    pub include_topics: Option<bool>,
    pub include_entities: Option<bool>,
    pub include_sources: Option<bool>,
    pub orphans_only: Option<bool>,
    pub query: Option<String>,
}

impl KnowledgeGraphData {
    /// Builds a graph from Scribbles and applies `filter` to the result.
    ///
    /// Equivalent to `build` followed by `filtered`, kept as one call for
    /// the callers that have no reason to hold the unfiltered graph.
    pub fn from_scribbles(scribbles: &[Scribble], filter: Option<&GraphFilter>) -> Self {
        let unfiltered = Self::build(scribbles);
        match filter {
            Some(f) => unfiltered.filtered(f),
            None => unfiltered,
        }
    }

    /// Builds the whole graph, filtering nothing.
    ///
    /// This is the expensive half — node construction and PageRank — and the
    /// half that only changes when the vault does, so it is what the index
    /// caches. Filtering runs against the cached result.
    pub fn build(scribbles: &[Scribble]) -> Self {
        let mut nodes_map: HashMap<String, KnowledgeNode> = HashMap::new();
        let mut edges: Vec<KnowledgeEdge> = Vec::new();
        let mut degrees: HashMap<String, usize> = HashMap::new();

        let scribble_ids: HashSet<String> = scribbles.iter().map(|s| s.id.clone()).collect();

        // 1. Create Scribble Nodes
        for scribble in scribbles {
            let label = if scribble.title.trim().is_empty() {
                "Untitled".to_string()
            } else {
                scribble.title.clone()
            };

            let snippet = scribble.summary.clone().unwrap_or_else(|| {
                let first_few: String = scribble.content.chars().take(140).collect();
                first_few
            });

            let node = KnowledgeNode {
                id: scribble.id.clone(),
                node_type: "scribble".to_string(),
                label,
                summary: Some(snippet),
                metadata: serde_json::json!({
                    "created_at": scribble.created_at,
                    "updated_at": scribble.updated_at,
                    "source_type": scribble.source_type,
                    "source_metadata": scribble.source_metadata,
                    "tags": scribble.tags,
                    "topics": scribble.topics,
                    "entities": scribble.entities,
                    "status": scribble.status,
                    "enrichment_status": scribble.ai_metadata.enrichment_status,
                }),
                degree: 0,
                source_type: Some(scribble.source_type.clone()),
                pagerank: 0.0,
                para: scribble.para,
                updated_at: Some(scribble.updated_at.clone()),
            };

            nodes_map.insert(scribble.id.clone(), node);
            degrees.insert(scribble.id.clone(), 0);

            // 2. Process Topic Nodes & Edges
            for topic in &scribble.topics {
                let topic_slug = slugify(topic);
                let topic_node_id = format!("topic_{}", topic_slug);

                nodes_map.entry(topic_node_id.clone()).or_insert_with(|| KnowledgeNode {
                    id: topic_node_id.clone(),
                    node_type: "topic".to_string(),
                    label: topic.clone(),
                    summary: Some(format!("Topic cluster for '{}'", topic)),
                    metadata: serde_json::json!({ "topic": topic }),
                    degree: 0,
                    source_type: None,
                    pagerank: 0.0,
                    para: None,
                    updated_at: None,
                });
                freshen(&mut nodes_map, &topic_node_id, &scribble.updated_at);

                edges.push(KnowledgeEdge {
                    id: format!("edge_{}_{}", scribble.id, topic_node_id),
                    source_id: scribble.id.clone(),
                    target_id: topic_node_id.clone(),
                    relationship: REL_SAME_TOPIC.to_string(),
                    confidence: 1.0,
                    source: "system".to_string(),
                });

                *degrees.entry(scribble.id.clone()).or_insert(0) += 1;
                *degrees.entry(topic_node_id).or_insert(0) += 1;
            }

            // 3. Process Entity Nodes & Edges
            for entity in &scribble.entities {
                let entity_slug = slugify(entity);
                let entity_node_id = format!("entity_{}", entity_slug);

                nodes_map.entry(entity_node_id.clone()).or_insert_with(|| KnowledgeNode {
                    id: entity_node_id.clone(),
                    node_type: "entity".to_string(),
                    label: entity.clone(),
                    summary: Some(format!("Entity mention for '{}'", entity)),
                    metadata: serde_json::json!({ "entity": entity }),
                    degree: 0,
                    source_type: None,
                    pagerank: 0.0,
                    para: None,
                    updated_at: None,
                });
                freshen(&mut nodes_map, &entity_node_id, &scribble.updated_at);

                edges.push(KnowledgeEdge {
                    id: format!("edge_{}_{}", scribble.id, entity_node_id),
                    source_id: scribble.id.clone(),
                    target_id: entity_node_id.clone(),
                    relationship: REL_MENTIONS.to_string(),
                    confidence: 1.0,
                    source: "system".to_string(),
                });

                *degrees.entry(scribble.id.clone()).or_insert(0) += 1;
                *degrees.entry(entity_node_id).or_insert(0) += 1;
            }

            // 4. Process Source Provenance Nodes & Edges
            if scribble.source_type == SOURCE_TYPE_VOICE {
                let mut vn_ids: Vec<String> = Vec::new();
                if let Some(arr) = scribble.source_metadata.get("source_voice_note_ids").and_then(|v| v.as_array()) {
                    for v in arr {
                        if let Some(s) = v.as_str() {
                            if !vn_ids.contains(&s.to_string()) {
                                vn_ids.push(s.to_string());
                            }
                        }
                    }
                }
                if vn_ids.is_empty() {
                    if let Some(vn_id) = scribble.source_metadata.get("source_voice_note_id").and_then(|v| v.as_str()) {
                        vn_ids.push(vn_id.to_string());
                    }
                }

                for vn_id in vn_ids {
                    let source_node_id = format!("source_{}", vn_id);
                    nodes_map.entry(source_node_id.clone()).or_insert_with(|| KnowledgeNode {
                        id: source_node_id.clone(),
                        node_type: "source".to_string(),
                        label: format!("Voice Note {}", vn_id.chars().take(12).collect::<String>()),
                        summary: Some("Originating Voice Note recording".to_string()),
                        metadata: serde_json::json!({ "voice_note_id": vn_id }),
                        degree: 0,
                        source_type: Some("voice".to_string()),
                        pagerank: 0.0,
                        para: None,
                        updated_at: None,
                    });
                    freshen(&mut nodes_map, &source_node_id, &scribble.updated_at);

                    edges.push(KnowledgeEdge {
                        id: format!("edge_{}_{}", scribble.id, source_node_id),
                        source_id: scribble.id.clone(),
                        target_id: source_node_id.clone(),
                        relationship: REL_DERIVED_FROM.to_string(),
                        confidence: 1.0,
                        source: "system".to_string(),
                    });

                    *degrees.entry(scribble.id.clone()).or_insert(0) += 1;
                    *degrees.entry(source_node_id).or_insert(0) += 1;
                }
            } else if scribble.source_type == SOURCE_TYPE_FILE {
                if let Some(filename) = scribble.source_metadata.get("filename").and_then(|v| v.as_str()) {
                    let source_node_id = format!("file_{}", slugify(filename));
                    nodes_map.entry(source_node_id.clone()).or_insert_with(|| KnowledgeNode {
                        id: source_node_id.clone(),
                        node_type: "source".to_string(),
                        label: filename.to_string(),
                        summary: Some("Source file document".to_string()),
                        metadata: serde_json::json!({ "filename": filename }),
                        degree: 0,
                        source_type: Some("file".to_string()),
                        pagerank: 0.0,
                        para: None,
                        updated_at: None,
                    });
                    freshen(&mut nodes_map, &source_node_id, &scribble.updated_at);

                    edges.push(KnowledgeEdge {
                        id: format!("edge_{}_{}", scribble.id, source_node_id),
                        source_id: scribble.id.clone(),
                        target_id: source_node_id.clone(),
                        relationship: REL_DERIVED_FROM.to_string(),
                        confidence: 1.0,
                        source: "system".to_string(),
                    });

                    *degrees.entry(scribble.id.clone()).or_insert(0) += 1;
                    *degrees.entry(source_node_id).or_insert(0) += 1;
                }
            }

            // 5. Process Explicit Relationships between Scribbles
            for rel in &scribble.relationships {
                if scribble_ids.contains(&rel.target_id) {
                    edges.push(KnowledgeEdge {
                        id: format!("edge_{}_{}", scribble.id, rel.target_id),
                        source_id: scribble.id.clone(),
                        target_id: rel.target_id.clone(),
                        relationship: rel.relationship_type.clone(),
                        confidence: rel.confidence,
                        source: rel.source.clone(),
                    });

                    *degrees.entry(scribble.id.clone()).or_insert(0) += 1;
                    *degrees.entry(rel.target_id.clone()).or_insert(0) += 1;
                }
            }
        }

        // Apply updated degrees to all nodes
        for (id, node) in nodes_map.iter_mut() {
            if let Some(deg) = degrees.get(id) {
                node.degree = *deg;
            }
        }

        // Rank before filtering. A node's structural importance is a property
        // of the whole vault, not of whichever slice is on screen — hiding
        // topics must not resize the scribbles that remain.
        let ranks = pagerank(&nodes_map, &edges);
        for (id, node) in nodes_map.iter_mut() {
            if let Some(r) = ranks.get(id) {
                node.pagerank = *r;
            }
        }

        let mut final_nodes: Vec<KnowledgeNode> = nodes_map.into_values().collect();
        let final_edges = edges;

        // Sort nodes stably
        final_nodes.sort_by(|a, b| a.label.cmp(&b.label));

        KnowledgeGraphData {
            nodes: final_nodes,
            edges: final_edges,
        }
    }

    /// A filtered copy of this graph.
    ///
    /// Only ever removes: no node is rebuilt, re-ranked or re-positioned by
    /// filtering, so a node that survives a filter is byte-identical to the
    /// one in the unfiltered graph. That is what lets the Rings view treat
    /// filtering as a change of opacity rather than a change of layout.
    pub fn filtered(&self, f: &GraphFilter) -> Self {
        let mut final_nodes: Vec<KnowledgeNode> = self.nodes.clone();
        let mut final_edges: Vec<KnowledgeEdge> = self.edges.clone();

        {
            if f.orphans_only.unwrap_or(false) {
                // An orphan is a scribble node with degree <= 1 (only itself or no inter-scribble/topic connections)
                // or degree == 0
                final_nodes.retain(|n| n.node_type == "scribble" && n.degree == 0);
                let orphan_ids: HashSet<String> = final_nodes.iter().map(|n| n.id.clone()).collect();
                final_edges.retain(|e| orphan_ids.contains(&e.source_id) && orphan_ids.contains(&e.target_id));
            } else {
                let inc_scribbles = f.include_scribbles.unwrap_or(true);
                let inc_topics = f.include_topics.unwrap_or(true);
                let inc_entities = f.include_entities.unwrap_or(true);
                let inc_sources = f.include_sources.unwrap_or(true);

                final_nodes.retain(|n| match n.node_type.as_str() {
                    "scribble" => inc_scribbles,
                    "topic" => inc_topics,
                    "entity" => inc_entities,
                    "source" => inc_sources,
                    _ => true,
                });

                if let Some(q) = &f.query {
                    let q_lower = q.to_lowercase().trim().to_string();
                    if !q_lower.is_empty() {
                        final_nodes.retain(|n| {
                            n.label.to_lowercase().contains(&q_lower)
                                || n.summary.as_ref().is_some_and(|s| s.to_lowercase().contains(&q_lower))
                        });
                    }
                }

                let remaining_ids: HashSet<String> = final_nodes.iter().map(|n| n.id.clone()).collect();
                final_edges.retain(|e| remaining_ids.contains(&e.source_id) && remaining_ids.contains(&e.target_id));
            }
        }

        KnowledgeGraphData {
            nodes: final_nodes,
            edges: final_edges,
        }
    }
}

/// Raises a derived node's `updated_at` to the newest contributor seen so far.
///
/// Topics and entities have no timestamp of their own. Taking the maximum
/// rather than the first writer's value is what makes a topic read as fresh
/// when any thought under it is fresh, which is what the Rings view's
/// recency temperature is claiming to show. RFC3339 strings from
/// `chrono::Utc` sort lexically in time order, so a string compare is a time
/// compare here.
fn freshen(nodes_map: &mut HashMap<String, KnowledgeNode>, node_id: &str, candidate: &str) {
    if let Some(node) = nodes_map.get_mut(node_id) {
        let newer = node
            .updated_at
            .as_ref()
            .is_none_or(|existing| candidate > existing.as_str());
        if newer {
            node.updated_at = Some(candidate.to_string());
        }
    }
}

/// Damping factor: the standard 0.85, the probability a walk follows an edge
/// rather than teleporting.
const PAGERANK_DAMPING: f32 = 0.85;

/// A fixed iteration count, deliberately not a convergence threshold.
///
/// An early exit on "close enough" makes the output depend on how the input
/// happened to be shaped, and the Rings view is built on the promise that
/// the same vault produces the same picture. 40 sweeps is past the point
/// where ranks move visibly at vault scale, and costs the same every time.
const PAGERANK_ITERATIONS: usize = 40;

/// PageRank over the knowledge graph, computed once per index build.
///
/// Edges are walked in both directions. The stored edges are directional
/// (`scribble -> topic`, `scribble -> source`), but that direction records
/// provenance rather than endorsement: treating it as one-way would sink all
/// the rank into topic and source nodes and leave every scribble at the
/// floor, which is the opposite of what node size should say.
///
/// Returns raw ranks summing to 1.0. Callers that need a drawing radius
/// normalise against the largest rank in view — that is a render-time
/// concern and does not change what was ranked here.
fn pagerank(
    nodes_map: &HashMap<String, KnowledgeNode>,
    edges: &[KnowledgeEdge],
) -> HashMap<String, f32> {
    let n = nodes_map.len();
    if n == 0 {
        return HashMap::new();
    }

    // Deterministic node order: iteration order of a HashMap is not stable
    // between runs, and float addition is not associative, so summing
    // contributions in map order would make ranks differ run to run.
    let mut ids: Vec<&String> = nodes_map.keys().collect();
    ids.sort();
    let position: HashMap<&str, usize> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();

    // Undirected adjacency, deduplicated: a repeated edge between the same
    // pair is the same link seen twice, not twice the endorsement.
    let mut neighbours: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut seen_pairs: HashSet<(usize, usize)> = HashSet::new();
    for edge in edges {
        let (Some(&a), Some(&b)) = (
            position.get(edge.source_id.as_str()),
            position.get(edge.target_id.as_str()),
        ) else {
            continue;
        };
        if a == b {
            continue;
        }
        let key = if a < b { (a, b) } else { (b, a) };
        if seen_pairs.insert(key) {
            neighbours[a].push(b);
            neighbours[b].push(a);
        }
    }

    let uniform = 1.0 / n as f32;
    let teleport = (1.0 - PAGERANK_DAMPING) / n as f32;
    let mut rank = vec![uniform; n];
    let mut next = vec![0.0f32; n];

    for _ in 0..PAGERANK_ITERATIONS {
        // Dangling nodes (no links at all) would leak their rank out of the
        // system; redistributing it uniformly is what keeps the vector
        // summing to 1.
        let dangling: f32 = (0..n)
            .filter(|&i| neighbours[i].is_empty())
            .map(|i| rank[i])
            .sum();
        let spread = PAGERANK_DAMPING * dangling / n as f32;

        for slot in next.iter_mut() {
            *slot = teleport + spread;
        }
        for i in 0..n {
            let out_degree = neighbours[i].len();
            if out_degree == 0 {
                continue;
            }
            let share = PAGERANK_DAMPING * rank[i] / out_degree as f32;
            for &j in &neighbours[i] {
                next[j] += share;
            }
        }
        std::mem::swap(&mut rank, &mut next);
    }

    ids.into_iter()
        .enumerate()
        .map(|(i, id)| (id.clone(), rank[i]))
        .collect()
}

fn slugify(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

/// Search result aggregation across knowledge objects.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct KnowledgeSearchResult {
    pub direct_matches: Vec<Scribble>,
    pub related_scribbles: Vec<Scribble>,
    pub matched_topics: Vec<String>,
    pub matched_entities: Vec<String>,
    pub total_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scribble_markdown_roundtrip() {
        let mut scribble = Scribble::new_text("This is my important idea for knowledge graphs.", Some("Connected Ideas"));
        scribble.topics = vec!["Knowledge".to_string(), "Graph".to_string()];
        scribble.entities = vec!["Relay".to_string(), "Rust".to_string()];
        scribble.summary = Some("A summary of connected thoughts.".to_string());
        scribble.relationships.push(ScribbleRelationship {
            id: "rel_1".to_string(),
            target_id: "scribble_target_123".to_string(),
            relationship_type: REL_RELATED_TO.to_string(),
            confidence: 0.95,
            source: "ai".to_string(),
        });

        let md = scribble.format_markdown();
        let parsed = Scribble::parse_markdown(&md).expect("Should parse back cleanly");

        assert_eq!(parsed.id, scribble.id);
        assert_eq!(parsed.title, scribble.title);
        assert_eq!(parsed.content, scribble.content);
        assert_eq!(parsed.topics, vec!["Knowledge", "Graph"]);
        assert_eq!(parsed.entities, vec!["Relay", "Rust"]);
        assert_eq!(parsed.summary, Some("A summary of connected thoughts.".to_string()));
        assert_eq!(parsed.relationships.len(), 1);
        assert_eq!(parsed.relationships[0].relationship_type, REL_RELATED_TO);
    }

    #[test]
    fn test_knowledge_graph_construction() {
        let mut s1 = Scribble::new_text("Working on Rust backend architecture", Some("Rust Backend"));
        s1.topics = vec!["Rust".to_string(), "Architecture".to_string()];

        let mut s2 = Scribble::new_text("Building React graph view for connected notes", Some("Graph View"));
        s2.topics = vec!["Architecture".to_string()];
        s2.relationships.push(ScribbleRelationship {
            id: "rel_s2_s1".to_string(),
            target_id: s1.id.clone(),
            relationship_type: REL_EXTENDS.to_string(),
            confidence: 0.9,
            source: "user".to_string(),
        });

        let scribbles = vec![s1.clone(), s2.clone()];
        let graph = KnowledgeGraphData::from_scribbles(&scribbles, None);

        // Nodes: 2 scribbles + 2 topics ("Rust", "Architecture") = 4 nodes
        assert_eq!(graph.nodes.len(), 4);
        // Topic "Architecture" should have degree = 2 (connected to s1 and s2)
        let arch_topic = graph.nodes.iter().find(|n| n.label == "Architecture").unwrap();
        assert_eq!(arch_topic.degree, 2);

        // Explicit edge from s2 to s1 should be present
        let explicit_edge = graph.edges.iter().find(|e| e.source_id == s2.id && e.target_id == s1.id);
        assert!(explicit_edge.is_some());
        assert_eq!(explicit_edge.unwrap().relationship, REL_EXTENDS);
    }

    #[test]
    fn test_orphan_node_filtering() {
        let s_connected = Scribble::new_text("Connected node", Some("Connected"));
        let mut s_connected2 = Scribble::new_text("Connected node 2", Some("Connected 2"));
        s_connected2.relationships.push(ScribbleRelationship {
            id: "rel_1".to_string(),
            target_id: s_connected.id.clone(),
            relationship_type: REL_RELATED_TO.to_string(),
            confidence: 1.0,
            source: "user".to_string(),
        });

        let s_orphan = Scribble::new_text("Isolated thought without any tags, topics or links", Some("Orphan Thought"));

        let scribbles = vec![s_connected, s_connected2, s_orphan.clone()];
        let filter = GraphFilter {
            orphans_only: Some(true),
            ..Default::default()
        };

        let graph = KnowledgeGraphData::from_scribbles(&scribbles, Some(&filter));
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].id, s_orphan.id);
    }
}
