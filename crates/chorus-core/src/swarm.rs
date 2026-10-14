//! Declarative Multi-Agent DAG Swarm Coordinator & Git Worktree Engine.
//!
//! Orchestrates concurrent subagent execution across isolated Git worktrees,
//! resolving dependencies via a Directed Acyclic Graph (DAG) and enforcing
//! strict per-subagent token and dollar spend circuit-breakers.

use crate::{ChorusError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Status of a swarm node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwarmNodeStatus {
    Pending,
    Ready,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// A node in the Agent Swarm DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmNode {
    pub id: String,
    pub role: String,
    pub task_prompt: String,
    pub model_id: Option<String>,
    pub depends_on: Vec<String>,
    pub status: SwarmNodeStatus,
    pub worktree_path: Option<PathBuf>,
    pub max_turns: u32,
    pub max_cost_usd: f64,
    pub current_cost_usd: f64,
}

/// The Swarm DAG coordinator.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SwarmDag {
    pub nodes: HashMap<String, SwarmNode>,
}

impl SwarmDag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_node(&mut self, node: SwarmNode) {
        self.nodes.insert(node.id.clone(), node);
    }

    /// Find nodes that are ready to run (all dependencies completed).
    pub fn find_ready_nodes(&self) -> Vec<String> {
        let mut ready = Vec::new();
        for (id, node) in &self.nodes {
            if node.status == SwarmNodeStatus::Pending || node.status == SwarmNodeStatus::Ready {
                let all_deps_done = node.depends_on.iter().all(|dep_id| {
                    self.nodes
                        .get(dep_id)
                        .map(|dep| dep.status == SwarmNodeStatus::Completed)
                        .unwrap_or(false)
                });
                if all_deps_done {
                    ready.push(id.clone());
                }
            }
        }
        ready
    }

    /// Mark a node as running and assign its worktree.
    pub fn mark_running(&mut self, node_id: &str, worktree_path: PathBuf) -> Result<()> {
        let node = self
            .nodes
            .get_mut(node_id)
            .ok_or_else(|| ChorusError::Invalid(format!("Node {} not found", node_id)))?;
        node.status = SwarmNodeStatus::Running;
        node.worktree_path = Some(worktree_path);
        Ok(())
    }

    /// Update cost and enforce circuit breaker.
    pub fn record_cost(&mut self, node_id: &str, cost_usd: f64) -> Result<bool> {
        let node = self
            .nodes
            .get_mut(node_id)
            .ok_or_else(|| ChorusError::Invalid(format!("Node {} not found", node_id)))?;
        node.current_cost_usd += cost_usd;

        if node.current_cost_usd > node.max_cost_usd {
            node.status = SwarmNodeStatus::Cancelled;
            return Ok(false); // Breached budget
        }
        Ok(true)
    }

    /// Mark a node as completed.
    pub fn mark_completed(&mut self, node_id: &str) -> Result<()> {
        let node = self
            .nodes
            .get_mut(node_id)
            .ok_or_else(|| ChorusError::Invalid(format!("Node {} not found", node_id)))?;
        node.status = SwarmNodeStatus::Completed;
        Ok(())
    }

    /// Check if all nodes in the DAG have completed.
    pub fn is_all_completed(&self) -> bool {
        !self.nodes.is_empty()
            && self
                .nodes
                .values()
                .all(|n| n.status == SwarmNodeStatus::Completed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swarm_dag_resolves_dependencies_in_order() {
        let mut dag = SwarmDag::new();

        // Node 1: Architect
        dag.add_node(SwarmNode {
            id: "arch".into(),
            role: "Architect".into(),
            task_prompt: "Plan".into(),
            model_id: None,
            depends_on: vec![],
            status: SwarmNodeStatus::Pending,
            worktree_path: None,
            max_turns: 5,
            max_cost_usd: 1.0,
            current_cost_usd: 0.0,
        });

        // Node 2: Coder (depends on arch)
        dag.add_node(SwarmNode {
            id: "coder".into(),
            role: "Coder".into(),
            task_prompt: "Code".into(),
            model_id: None,
            depends_on: vec!["arch".into()],
            status: SwarmNodeStatus::Pending,
            worktree_path: None,
            max_turns: 10,
            max_cost_usd: 2.0,
            current_cost_usd: 0.0,
        });

        // Initially only arch is ready
        assert_eq!(dag.find_ready_nodes(), vec!["arch".to_string()]);

        // Complete arch
        dag.mark_completed("arch").unwrap();

        // Now coder is ready
        assert_eq!(dag.find_ready_nodes(), vec!["coder".to_string()]);

        // Test budget governor
        dag.mark_running("coder", PathBuf::from("/tmp/wt")).unwrap();
        let ok = dag.record_cost("coder", 2.50).unwrap(); // exceeds 2.0
        assert!(!ok);
        assert_eq!(dag.nodes["coder"].status, SwarmNodeStatus::Cancelled);
    }
}
