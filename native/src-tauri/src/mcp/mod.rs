//! MCP (Model Context Protocol) Integration Layer.
//!
//! Exposes Vox's unified knowledge architecture (Retrieval, Context Assembly,
//! Context Pack, Memory, and Relationships) to MCP clients while enforcing the same
//! confirmation boundaries on mutations as the native UI.
//!
//! There is no outbound MCP client here. Actions against external services
//! (calendar, Notion, Drive) were once stubbed in this module as functions that
//! reported success without doing anything; they were removed rather than left
//! to lie, and are tracked in `maybe_later.md` ("Voice triggers that run
//! actions").

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::actions::{ActionDispatcher, UniversalAction};
use crate::context::{ContextAssemblyRequest, ContextAssemblyService, ContextPack};
use crate::memory::MemoryStore;
use crate::relationships::RelationshipStore;
use crate::vault::VaultManager;

#[derive(Error, Debug)]
pub enum McpError {
    #[error("MCP client error: {0}")]
    ClientError(String),

    #[error("Tool execution failed: {0}")]
    ToolExecutionFailed(String),

    #[error("Confirmation required: {0}")]
    ConfirmationRequired(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolCallResult {
    pub tool_name: String,
    pub success: bool,
    pub result_summary: String,
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
}

pub struct McpRouter;

impl McpRouter {
    /// Assembles a shared ContextPack for an external MCP query.
    pub fn assemble_context(
        vault: &VaultManager,
        memory_store: Option<&MemoryStore>,
        relationship_store: Option<&RelationshipStore>,
        query: &str,
        char_budget: usize,
    ) -> Result<ContextPack, McpError> {
        let req = ContextAssemblyRequest::new(query).with_char_budget(char_budget);
        Ok(ContextAssemblyService::assemble(vault, memory_store, relationship_store, &req))
    }

    /// Dispatches a universal action on behalf of an MCP caller with identical confirmation gating.
    pub fn execute_action(
        mut action: UniversalAction,
        confirmed: bool,
        vault: Option<&VaultManager>,
    ) -> Result<McpToolCallResult, McpError> {
        let tool_name = action.action_type.as_str().to_string();
        let res = ActionDispatcher::execute(&mut action, confirmed, vault)
            .map_err(|e| {
                if action.status == crate::actions::ActionStatus::RequiresConfirmation {
                    McpError::ConfirmationRequired(e)
                } else {
                    McpError::ToolExecutionFailed(e)
                }
            })?;

        Ok(McpToolCallResult {
            tool_name,
            success: true,
            result_summary: format!("Executed action '{}' successfully", action.action_type.as_str()),
            payload: Some(res),
        })
    }
}
