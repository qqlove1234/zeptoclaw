# Agent Swarm Design: DelegateTool

**Date**: 2026-02-13
**Status**: Approved

## Problem

ZeptoClaw has no multi-agent capability. NanoClaw achieves swarms by enabling
the Claude Agent SDK's built-in `TeamCreate`/`SendMessage` tools, but ZeptoClaw
uses its own `AgentLoop` calling the Claude API directly. We need a native swarm
mechanism built on ZeptoClaw's existing primitives.

## Decision

Add a `DelegateTool` — a single new tool that creates a temporary `AgentLoop`
with a role-specific system prompt and tool whitelist, runs it to completion,
and returns the result as a tool output to the calling (lead) agent.

The LLM's existing tool loop handles orchestration: the lead agent decides when
and how to decompose tasks, delegates via tool calls, and synthesizes results.

## Design

### Tool Interface

```json
{
  "name": "delegate",
  "description": "Delegate a task to a specialist sub-agent with a specific role",
  "parameters": {
    "role": "string — the specialist role (e.g., 'Researcher', 'Writer')",
    "task": "string — the task to complete",
    "tools": "string[] — optional tool whitelist override"
  }
}
```

### Data Flow

```
User message → Lead Agent (full tool set)
                    ↓
              delegate(role: "Researcher", task: "Find AI trends")
                    ↓
              DelegateTool.execute():
                1. Look up role in config.swarm.roles (optional presets)
                2. Build system prompt (from config or generated)
                3. Create temp AgentLoop with role's tool whitelist
                4. process_message() → full tool loop runs
                5. Return result string to lead agent
                    ↓
              Lead Agent receives result as tool output
                    ↓
              delegate(role: "Writer", task: "Summarize: {research}")
                    ↓
              (same flow)
                    ↓
              Lead Agent synthesizes final response → User
```

### Configuration

Optional role presets in config.json:

```json
{
  "swarm": {
    "enabled": true,
    "max_depth": 1,
    "max_concurrent": 3,
    "roles": {
      "researcher": {
        "system_prompt": "You are a research specialist. Be thorough and cite sources.",
        "tools": ["web_search", "web_fetch", "memory_search", "memory_get"]
      },
      "writer": {
        "system_prompt": "You are a technical writer. Be clear and concise.",
        "tools": ["read_file", "write_file", "edit_file", "memory_search"]
      }
    }
  }
}
```

Ad-hoc roles (not in config) get:
- System prompt: `"You are a {role}. Complete the following task."`
- Tools: all registered tools minus `delegate` and `spawn`

### Direct User Messaging

Sub-agents have access to `MessageTool`. When a sub-agent sends a message, it
is prefixed with the role name:

```
[Researcher]: Found an interesting pattern in the data...
```

This uses the existing bus `publish_outbound()` path. The prefix is injected by
the DelegateTool when constructing the sub-agent's tool context.

### Recursion & Safety

- `ctx.channel == "delegate"` blocks DelegateTool execution (prevents recursion)
- `max_depth: 1` — no sub-sub-agents (configurable for future)
- Sub-agents use isolated sessions (`delegate:{uuid}`) — no cross-contamination
- Timeout inherited from agent config defaults
- `max_concurrent: 3` — limits total active sub-agents (for future parallel mode)

### Sub-Agent Lifecycle

1. DelegateTool creates a fresh `AgentLoop` (no session history)
2. Registers only the tools allowed for the role
3. Sets a role-specific system prompt via `ContextBuilder`
4. Calls `process_message()` with `InboundMessage { channel: "delegate" }`
5. Collects the response string
6. Temp AgentLoop is dropped (no persistent state)

### Session Isolation

Sub-agents use ephemeral sessions keyed by `delegate:{uuid}`. They do NOT share
the lead agent's session or conversation history. The lead agent passes context
explicitly via the `task` parameter.

## Files

| File | Change |
|------|--------|
| `src/tools/delegate.rs` | New: DelegateTool (~200 lines) |
| `src/tools/mod.rs` | Add `pub mod delegate;` export |
| `src/config/types.rs` | Add `SwarmConfig`, `SwarmRole` structs |
| `src/main.rs` | Register DelegateTool in `create_agent()` |
| `tests/integration.rs` | Delegation integration tests |

## Scope Boundaries

**In scope (v1):**
- Single DelegateTool with role/task/tools parameters
- Config-based role presets with tool whitelists
- Ad-hoc roles with generated prompts
- Direct user messaging via MessageTool
- Recursion blocking
- Sequential execution

**Out of scope (future):**
- Parallel delegation (`tokio::spawn` + `join_all`)
- Shared context/memory between sub-agents
- Persistent sub-agent sessions
- Telegram pool bot identity (per-agent bot names)
- Sub-agent progress streaming
- Cost/token budgets per delegation

## Alternatives Considered

**B: SwarmOrchestrator module** — New `src/swarms/` with orchestrator, pool,
aggregator. Rejected: over-engineered (~800+ lines), adds abstraction layer
when the LLM already handles task decomposition via tool calls.

**C: Enhanced SpawnTool** — Add synchronous mode to existing SpawnTool.
Rejected: overloads SpawnTool's fire-and-forget purpose, messy parameter space.
