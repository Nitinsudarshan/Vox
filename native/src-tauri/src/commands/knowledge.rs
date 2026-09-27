//! Knowledge: retrieval, context packs, memories, decisions, entities, relationships and actions.

use super::*;

#[tauri::command]
pub async fn unified_retrieve(
    query: crate::retrieval::RetrievalQuery,
    state: State<'_, AppState>,
) -> Result<crate::retrieval::RetrievalResult, CommandError> {
    Ok(crate::retrieval::UnifiedRetrievalService::search_with_memory(
        &state.vault,
        Some(&state.memory_store),
        &query,
    ))
}

#[tauri::command]
pub async fn assemble_context_pack(
    query: String,
    pack_type: Option<String>,
    char_budget: Option<usize>,
    state: State<'_, AppState>,
) -> Result<crate::context::ContextPack, CommandError> {
    let pt = pack_type.map(|t| match t.to_lowercase().as_str() {
        "repository" => crate::context::ContextPackType::Repository,
        "project" => crate::context::ContextPackType::Project,
        "conversation" => crate::context::ContextPackType::Conversation,
        "document" => crate::context::ContextPackType::Document,
        _ => crate::context::ContextPackType::General,
    });

    let mut req = crate::context::ContextAssemblyRequest::new(&query);
    if let Some(t) = pt {
        req = req.with_pack_type(t);
    }
    if let Some(b) = char_budget {
        req = req.with_char_budget(b);
    }

    Ok(crate::context::ContextAssemblyService::assemble_full(
        &state.vault,
        Some(&state.memory_store),
        Some(&state.relationship_store),
        Some(&state.entity_store),
        &req,
    ))
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KnowledgeTelemetrySnapshot {
    pub total_memories: usize,
    pub active_memories: usize,
    pub total_entities: usize,
    pub total_relationships: usize,
    pub total_scribbles: usize,
    pub total_notes: usize,
    pub total_files: usize,
    pub total_captures: usize,
}

#[tauri::command]
pub async fn get_knowledge_telemetry(
    state: State<'_, AppState>,
) -> Result<KnowledgeTelemetrySnapshot, CommandError> {
    let active_mems = state.memory_store.list_active(None).len();
    let all_rels = state.relationship_store.list_all().len();
    let all_ents = state.entity_store.list_all().len();
    let scribbles = state.vault.list_scribbles().map(|v| v.len()).unwrap_or(0);
    let notes = state.vault.list_notes().map(|v| v.len()).unwrap_or(0);
    let (files, caps) = state.vault.list_vault_files()
        .map(|all| {
            let caps = all.iter().filter(|f| f.is_capture()).count();
            let files = all.len() - caps;
            (files, caps)
        })
        .unwrap_or((0, 0));

    Ok(KnowledgeTelemetrySnapshot {
        total_memories: active_mems,
        active_memories: active_mems,
        total_entities: all_ents,
        total_relationships: all_rels,
        total_scribbles: scribbles,
        total_notes: notes,
        total_files: files,
        total_captures: caps,
    })
}

#[tauri::command]
pub async fn form_memory_candidate(
    candidate: crate::memory::CandidateMemory,
    state: State<'_, AppState>,
) -> Result<crate::memory::MemoryFormationOutcome, CommandError> {
    crate::memory::MemoryFormationService::process_candidate(&state.memory_store, candidate)
        .map_err(|e| CommandError::new("MEMORY_ERROR", &e))
}

/// Every decision, superseded ones included — the tree keeps what was
/// reversed.
#[tauri::command]
pub async fn list_decisions(
    state: State<'_, AppState>,
) -> Result<Vec<crate::memory::DecisionRecord>, CommandError> {
    Ok(crate::memory::list_decisions(&state.memory_store))
}

/// Records a decision at the standing its provenance earns.
///
/// Confidence is not a parameter: it follows from `provenance`, and a
/// captured or extracted decision without a traceable source is refused
/// rather than downgraded.
#[tauri::command]
pub async fn record_decision(
    decision: crate::memory::NewDecision,
    state: State<'_, AppState>,
) -> Result<crate::memory::DecisionRecord, CommandError> {
    crate::memory::record_decision(&state.memory_store, decision)
        .map_err(|e| CommandError::new("DECISION_ERROR", &e.to_string()))
}

/// Promotes a decision one rung. A real write, not a UI state.
#[tauri::command]
pub async fn confirm_decision(
    id: String,
    source_id: String,
    evidence: String,
    state: State<'_, AppState>,
) -> Result<crate::memory::DecisionRecord, CommandError> {
    crate::memory::confirm_decision(&state.memory_store, &id, &source_id, &evidence)
        .map_err(|e| CommandError::new("DECISION_ERROR", &e.to_string()))
}

/// Replaces a decision with a later one, keeping both linked.
#[tauri::command]
pub async fn supersede_decision(
    old_id: String,
    decision: crate::memory::NewDecision,
    state: State<'_, AppState>,
) -> Result<Vec<crate::memory::DecisionRecord>, CommandError> {
    let (old, new) = crate::memory::supersede_decision(&state.memory_store, &old_id, decision)
        .map_err(|e| CommandError::new("DECISION_ERROR", &e.to_string()))?;
    Ok(vec![old, new])
}

#[tauri::command]
pub async fn list_entities(
    category: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<crate::entities::ResolvedEntity>, CommandError> {
    if let Some(cat_str) = category {
        if let Some(cat) = crate::entities::EntityCategory::from_str_opt(&cat_str) {
            return Ok(state.entity_store.list_by_category(cat));
        }
    }
    Ok(state.entity_store.list_all())
}

#[tauri::command]
pub async fn list_memories(
    memory_type: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<crate::memory::MemoryItem>, CommandError> {
    let mt = memory_type.and_then(|t| match t.to_lowercase().as_str() {
        "fact" => Some(crate::memory::MemoryType::Fact),
        "preference" => Some(crate::memory::MemoryType::Preference),
        "decision" => Some(crate::memory::MemoryType::Decision),
        "project_context" => Some(crate::memory::MemoryType::ProjectContext),
        "relationship" => Some(crate::memory::MemoryType::Relationship),
        "instruction" => Some(crate::memory::MemoryType::Instruction),
        _ => None,
    });
    Ok(state.memory_store.list_active(mt))
}

#[tauri::command]
pub async fn create_memory(
    memory_type: String,
    subject: String,
    content: String,
    source_id: String,
    evidence: String,
    state: State<'_, AppState>,
) -> Result<crate::memory::MemoryItem, CommandError> {
    let mt = match memory_type.to_lowercase().as_str() {
        "preference" => crate::memory::MemoryType::Preference,
        "decision" => crate::memory::MemoryType::Decision,
        "project_context" => crate::memory::MemoryType::ProjectContext,
        "relationship" => crate::memory::MemoryType::Relationship,
        "instruction" => crate::memory::MemoryType::Instruction,
        _ => crate::memory::MemoryType::Fact,
    };
    let prov = crate::memory::MemoryProvenance {
        source_id,
        source_type: "manual".to_string(),
        evidence,
        confidence: 1.0,
        extracted_by: "user".to_string(),
    };
    let item = crate::memory::MemoryItem::new(mt, subject, content, prov);
    state.memory_store.create_memory(item).map_err(|e| CommandError::new("MEMORY_ERROR", &e))
}

#[tauri::command]
pub async fn supersede_memory(
    old_id: String,
    new_content: String,
    source_id: String,
    evidence: String,
    state: State<'_, AppState>,
) -> Result<crate::memory::MemoryItem, CommandError> {
    let prov = crate::memory::MemoryProvenance {
        source_id,
        source_type: "update".to_string(),
        evidence,
        confidence: 1.0,
        extracted_by: "user".to_string(),
    };
    let (_old, new_mem) = state
        .memory_store
        .supersede_memory(&old_id, &new_content, prov)
        .map_err(|e| CommandError::new("MEMORY_ERROR", &e))?;
    Ok(new_mem)
}

#[tauri::command]
pub async fn extract_and_resolve_entities(
    source_id: String,
    content: String,
) -> Result<Vec<crate::entities::ResolvedEntity>, CommandError> {
    let extracted = crate::entities::EntityExtractor::extract_deterministic(&source_id, &content);
    Ok(crate::entities::EntityResolver::resolve(&extracted))
}

#[tauri::command]
pub async fn dispatch_universal_action(
    mut action: crate::actions::UniversalAction,
    confirmed: bool,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, CommandError> {
    crate::actions::ActionDispatcher::execute(&mut action, confirmed, Some(&state.vault))
        .map_err(|e| CommandError::new("ACTION_ERROR", &e))
}

#[tauri::command]
pub async fn list_relationships(
    source_id: Option<String>,
    target_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<crate::relationships::RelationshipRecord>, CommandError> {
    if let Some(s) = source_id {
        Ok(state.relationship_store.get_relationships_for_source(&s))
    } else if let Some(t) = target_id {
        Ok(state.relationship_store.get_relationships_for_target(&t))
    } else {
        Ok(state.relationship_store.list_all())
    }
}

#[tauri::command]
pub async fn add_relationship(
    source_id: String,
    target_id: String,
    relationship_type: String,
    state: State<'_, AppState>,
) -> Result<crate::relationships::RelationshipRecord, CommandError> {
    let rt = crate::relationships::RelationshipType::from_str_opt(&relationship_type)
        .ok_or_else(|| CommandError::new("INVALID_INPUT", &format!("Unknown relationship type: {}", relationship_type)))?;
    let rel = crate::relationships::RelationshipRecord::new(source_id, target_id, rt)
        .map_err(|e| CommandError::new("INVALID_INPUT", &e))?;
    state.relationship_store.add_relationship(rel.clone())
        .map_err(|e| CommandError::new("RELATIONSHIP_ERROR", &e))?;
    Ok(rel)
}
