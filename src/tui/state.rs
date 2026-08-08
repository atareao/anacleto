//! State helpers for the TUI: sidebar panel counts and fuzzy matching.

use crate::agent::types::AgentStatus;
use crate::tui::app::App;

impl App {
    /// Number of unique MCP servers shown in the MCPs sidebar panel.
    pub(crate) fn unique_mcp_count(&self) -> usize {
        self.agents
            .iter()
            .flat_map(|a| a.mcps.iter())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    }

    /// Number of unique skills shown in the Skills sidebar panel.
    pub(crate) fn unique_skill_count(&self) -> usize {
        self.agents
            .iter()
            .flat_map(|a| a.skills.iter())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    }

    /// Number of agents shown in the Agents sidebar panel (non-completed).
    pub(crate) fn agent_panel_count(&self) -> usize {
        self.agents
            .iter()
            .filter(|a| a.status != AgentStatus::Completed)
            .count()
    }

    /// Number of unique subagent names configured across all root agents.
    pub(crate) fn unique_subagent_count(&self) -> usize {
        self.configured_subagents
            .values()
            .flat_map(|v| v.iter())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    }
}

/// Fuzzy-match `query` against `candidate` (case-insensitive subsequence).
/// Returns a score if every character of `query` appears in order in
/// `candidate`, or `None` otherwise. Higher scores rank better: consecutive
/// matches, matches near the start, and shorter candidates are preferred.
pub(crate) fn fuzzy_score(query: &str, candidate: &str) -> Option<u32> {
    let q: Vec<char> = query.chars().flat_map(|c| c.to_lowercase()).collect();
    let c: Vec<char> = candidate.chars().flat_map(|c| c.to_lowercase()).collect();
    if q.is_empty() {
        return Some(0);
    }

    let mut qi = 0;
    let mut score = 0u32;
    let mut prev: Option<usize> = None;
    for (ci, &ch) in c.iter().enumerate() {
        if qi < q.len() && ch == q[qi] {
            // Bonus for consecutive matches.
            score += match prev {
                Some(p) if ci == p + 1 => 8,
                _ => 2,
            };
            // Bonus for a match at the very start of the candidate.
            if ci == 0 {
                score += 5;
            }
            prev = Some(ci);
            qi += 1;
        }
    }
    if qi == q.len() {
        // Prefer shorter candidates (fewer extra characters).
        score += (c.len() as u32).saturating_sub(q.len() as u32).max(1);
        Some(score)
    } else {
        None
    }
}
