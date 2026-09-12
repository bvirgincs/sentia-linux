# Decisions

## Decision log

| ID | Decision | Rationale | Rejected alternatives |
| --- | --- | --- | --- |
| D-001 | Keep this checkpoint docs-only. | The worktree is constrained to foundation artifacts and honest state reporting. | Claiming runtime progress or changing build, code, packaging, or AWS paths. |
| D-002 | License original Sentia work under Apache-2.0. | Matches the approved plan and keeps future original contributions permissive. | Reintroducing a copyleft-only top-level license. |
| D-003 | Preserve upstream licenses and corresponding-source obligations. | Debian, Calamares, and apt-linked material are not original Sentia work. | Relabeling all repository contents as Apache-only. |
| D-004 | Use supported `.github/agents/*.agent.md` custom-agent frontmatter. | Matches the current Copilot custom-agent format and keeps profiles portable. | Storing profiles in unsupported locations or using undocumented headers. |
| D-005 | Treat acceptance as machine-readable intent, not as success until evidence exists. | Prevents drift between documentation and actual test results. | Marking unrun build, VM, installer, or security checks as complete. |

## Notes

These decisions are checkpoint-specific and may be refined as implementation
progresses, but they must remain honest about what has and has not been built.
