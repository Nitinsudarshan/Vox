//! Memory layer module.
//!
//! Provides long-term, maintainable, provenance-grounded memory with explicit
//! lifecycles.

pub mod decision;
pub mod formation;
pub mod model;
pub mod store;

pub use decision::{
    confirm_decision, list_decisions, record_decision, supersede_decision, DecisionEvidence,
    DecisionProvenance, DecisionRecord, NewDecision,
};
pub use formation::{CandidateMemory, FormationAction, MemoryFormationOutcome, MemoryFormationService};
pub use model::{EpistemicState, MemoryItem, MemoryProvenance, MemoryStatus, MemoryType};
pub use store::MemoryStore;

