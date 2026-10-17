//! Chorus core — ledger, blob store, memory and context assembly.
//!
//! Design rule that governs every module here (see the Storage Spec):
//! **what we keep on disk and what we send to a model are different formats.**
//! Cold storage optimises for bytes and query speed; the hot context is
//! assembled fresh on every call and never written back.

pub mod actor;
pub mod agent_control;
pub mod attachments;
pub mod blob;
pub mod checkpoint;
pub mod checkpoint_dag;
pub mod compaction;
pub mod context;
pub mod darwin_qos;
pub mod diff_align;
pub mod error;
pub mod file_tree;
pub mod handoff;
pub mod highlight;
pub mod hooks;
pub mod inference;
pub mod integration_catalog;
pub mod ledger;
pub mod local_router;
pub mod lsp_client;
pub mod mcp;
pub mod mcp_config;
pub mod mcp_host;
pub mod memory;
pub mod merge_specialist;
pub mod merkle;
pub mod mlx_runtime;
pub mod model_manager;
pub mod notify;
pub mod permissions;
pub mod pipeline;
pub mod projector;
pub mod render;
pub mod repo_map;
pub mod review;
pub mod roles;
pub mod router;
pub mod schema;
pub mod search;
pub mod skills;
pub mod store;
pub mod structural_diff;
pub mod swarm;
pub mod tokens;
pub mod types;
pub mod updater;
pub mod vector_accelerate;
pub mod verify_loop;
pub mod worktree_cow;
pub mod worktrees;

pub use agent_control::{
    ChecklistItem, FileJump, ReviewRequest, Subagent, SubagentBrief, SubagentResultsBrief,
    SubagentStatus,
};
pub use checkpoint_dag::{CheckpointDag, RewindMode, RewindSummary, TurnCheckpointRecord};
pub use darwin_qos::DarwinQoS;
pub use error::{ChorusError, Result};
pub use merge_specialist::{ConflictHunk, MergeSpecialist, MergeStrategy};
pub use merkle::{MerkleDiff, MerkleNode, MerkleTree};
pub use mlx_runtime::{
    AppleSiliconHardwareInfo, AppleSiliconMlxEngine, MlxInferenceStats, MlxModelConfig,
    QuantizationType,
};
pub use pipeline::{ArchitectEditorPipeline, BudgetPolicy, PipelineCostSummary};
pub use repo_map::{RepoMapEngine, SymbolDef, SymbolExtractor, SymbolGraph};
pub use roles::AgentRole;
pub use store::Store;
pub use structural_diff::{ChangeSignificance, StructuralDiff, StructuralDiffAnalysis};
pub use swarm::{SwarmDag, SwarmNode, SwarmNodeStatus};
pub use types::{BlobId, Event, EventKind, NewEvent, Rung};
pub use updater::{evaluate_update, SemVer, UpdateCheckResult};
pub use vector_accelerate::VectorOps;
pub use verify_loop::{DiagnosticItem, VerificationResult, VerifyLoop};
pub use worktree_cow::WorktreeCow;
