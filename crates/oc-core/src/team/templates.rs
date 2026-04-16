use std::sync::Arc;

use crate::session::SessionDB;
use super::types::*;

/// Return built-in + user-saved templates (business logic lives in oc-core).
pub fn all_templates(db: &Arc<SessionDB>) -> Vec<TeamTemplate> {
    let mut templates = builtin_templates();
    if let Ok(user) = db.list_team_templates() {
        templates.extend(user);
    }
    templates
}

/// Return the 4 built-in team templates.
pub fn builtin_templates() -> Vec<TeamTemplate> {
    vec![
        TeamTemplate {
            template_id: "builtin-fullstack".to_string(),
            name: "Full-Stack Feature".to_string(),
            description: "Frontend + Backend + Tester working in parallel on a full-stack feature"
                .to_string(),
            builtin: true,
            members: vec![
                TeamTemplateMember {
                    name: "Frontend".to_string(),
                    role: MemberRole::Worker,
                    agent_id: "default".to_string(),
                    color: "#3B82F6".to_string(),
                    description: "Build React frontend components and pages".to_string(),
                },
                TeamTemplateMember {
                    name: "Backend".to_string(),
                    role: MemberRole::Worker,
                    agent_id: "default".to_string(),
                    color: "#10B981".to_string(),
                    description: "Implement API endpoints and backend logic".to_string(),
                },
                TeamTemplateMember {
                    name: "Tester".to_string(),
                    role: MemberRole::Reviewer,
                    agent_id: "default".to_string(),
                    color: "#F59E0B".to_string(),
                    description: "Write integration tests and verify correctness".to_string(),
                },
            ],
        },
        TeamTemplate {
            template_id: "builtin-code-review".to_string(),
            name: "Code Review".to_string(),
            description: "Writer implements, Reviewer reviews, Tester verifies".to_string(),
            builtin: true,
            members: vec![
                TeamTemplateMember {
                    name: "Writer".to_string(),
                    role: MemberRole::Worker,
                    agent_id: "default".to_string(),
                    color: "#3B82F6".to_string(),
                    description: "Write the implementation code".to_string(),
                },
                TeamTemplateMember {
                    name: "Reviewer".to_string(),
                    role: MemberRole::Reviewer,
                    agent_id: "default".to_string(),
                    color: "#8B5CF6".to_string(),
                    description: "Review code quality, patterns, and correctness".to_string(),
                },
                TeamTemplateMember {
                    name: "Tester".to_string(),
                    role: MemberRole::Reviewer,
                    agent_id: "default".to_string(),
                    color: "#F59E0B".to_string(),
                    description: "Write tests and verify functionality".to_string(),
                },
            ],
        },
        TeamTemplate {
            template_id: "builtin-research-implement".to_string(),
            name: "Research & Implement".to_string(),
            description: "Researcher gathers information, Implementer codes the solution"
                .to_string(),
            builtin: true,
            members: vec![
                TeamTemplateMember {
                    name: "Researcher".to_string(),
                    role: MemberRole::Worker,
                    agent_id: "default".to_string(),
                    color: "#06B6D4".to_string(),
                    description: "Research codebase, docs, and best practices".to_string(),
                },
                TeamTemplateMember {
                    name: "Implementer".to_string(),
                    role: MemberRole::Worker,
                    agent_id: "default".to_string(),
                    color: "#10B981".to_string(),
                    description: "Implement the solution based on research findings".to_string(),
                },
            ],
        },
        TeamTemplate {
            template_id: "builtin-large-refactor".to_string(),
            name: "Large Refactor".to_string(),
            description:
                "Analyst plans the refactoring, two Refactorers execute, Tester verifies"
                    .to_string(),
            builtin: true,
            members: vec![
                TeamTemplateMember {
                    name: "Analyst".to_string(),
                    role: MemberRole::Worker,
                    agent_id: "default".to_string(),
                    color: "#8B5CF6".to_string(),
                    description: "Analyze the codebase and plan the refactoring strategy"
                        .to_string(),
                },
                TeamTemplateMember {
                    name: "Refactorer-1".to_string(),
                    role: MemberRole::Worker,
                    agent_id: "default".to_string(),
                    color: "#3B82F6".to_string(),
                    description: "Execute refactoring on assigned modules".to_string(),
                },
                TeamTemplateMember {
                    name: "Refactorer-2".to_string(),
                    role: MemberRole::Worker,
                    agent_id: "default".to_string(),
                    color: "#10B981".to_string(),
                    description: "Execute refactoring on assigned modules".to_string(),
                },
                TeamTemplateMember {
                    name: "Tester".to_string(),
                    role: MemberRole::Reviewer,
                    agent_id: "default".to_string(),
                    color: "#F59E0B".to_string(),
                    description: "Run tests continuously and verify no regressions".to_string(),
                },
            ],
        },
    ]
}
