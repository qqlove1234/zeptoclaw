# ZeptoClaw Roadmap

> Last updated: 2026-02-14

## Process

After completing any feature, update these 3 files:
1. **This file** (`docs/plans/TODO.md`) — check off items, update "Last updated" date
2. **`CLAUDE.md`** — architecture tree, test counts, module docs, CLI flags
3. **`AGENTS.md`** — "Current State" section, test counts, "Not Yet Wired" list

---

## Completed

### Quick Wins (2026-02-13)
- [x] Tool result sanitization (`src/utils/sanitize.rs`) — strips base64, hex blobs, truncates to 50KB
- [x] Parallel tool execution (`src/agent/loop.rs`) — `futures::future::join_all`
- [x] Agent-level timeout — `agent_timeout_secs` config field (default 300s)
- [x] Config validation CLI (`src/config/validate.rs`) — `zeptoclaw config check`
- [x] Message queue modes — Collect (default) and Followup for busy sessions

### Agent Swarm (2026-02-13)
- [x] SwarmConfig + SwarmRole structs (`src/config/types.rs`)
- [x] DelegateTool (`src/tools/delegate.rs`) — creates sub-agent with role-specific prompt + tool whitelist
- [x] Recursion blocking via channel check
- [x] ProviderRef wrapper for shared provider
- [x] Wired into `create_agent()` after provider resolution

### Streaming Responses (2026-02-14)
- [x] StreamEvent enum + `chat_stream()` default on LLMProvider trait
- [x] Claude SSE streaming (`src/providers/claude.rs`)
- [x] OpenAI SSE streaming (`src/providers/openai.rs`)
- [x] Streaming config field + `--stream` CLI flag
- [x] ProviderRef `chat_stream()` forwarding for delegate tool
- [x] `process_message_streaming()` on AgentLoop
- [x] CLI output wiring (single-message + interactive modes)
- [x] Integration tests

### Provider Infrastructure (2026-02-14)
- [x] RetryProvider (`src/providers/retry.rs`) — exponential backoff on 429/5xx
- [x] FallbackProvider (`src/providers/fallback.rs`) — primary → secondary auto-failover
- [x] MetricsCollector (`src/utils/metrics.rs`) — tool call stats, token tracking, session summary

### Wiring (2026-02-14)
- [x] Config fields for retry/fallback (`config/types.rs`, `config/mod.rs` env overrides)
- [x] RetryProvider wired into provider resolution (`main.rs`) — base → fallback → retry stack
- [x] FallbackProvider wired with multi-provider resolution (`providers/registry.rs`)
- [x] MetricsCollector wired into AgentLoop — tracks tool duration/success + token usage
- [x] Status output shows retry/fallback state

### Agent Loop / CLI Wiring (2026-02-14)
- [x] ConversationHistory CLI commands — `history list`, `history show <query>`, `history cleanup` in `main.rs`
- [x] TokenBudget wired — `token_budget` config field + env override + budget check in agent loop
- [x] OutputFormat wired — `output_format` field on `ChatOptions` + OpenAI `response_format` + Claude system suffix
- [x] LongTermMemory tool — `longterm_memory` agent tool (set/get/search/delete/list/categories), 22 tests

## Backlog — Next Features

### High Priority (2026-02-14)
- [x] **Conversation persistence** (`src/session/history.rs`) — CLI session discovery, listing, search, cleanup (12 tests)
- [x] **Token budget / rate limiting** (`src/agent/budget.rs`) — atomic per-session token budget tracker (18 tests)
- [x] **Structured output** (`src/providers/structured.rs`) — OutputFormat enum (Text/Json/JsonSchema) with provider helpers (19 tests)
- [x] **Multi-turn memory** (`src/memory/longterm.rs`) — persistent key-value store with categories, tags, access tracking (19 tests)

### Medium Priority (2026-02-14)
- [x] **Webhook channel** (`src/channels/webhook.rs`) — HTTP POST inbound channel with auth, wired in factory (28 tests)
- [x] **Discord channel** (`src/channels/discord.rs`) — Gateway WebSocket + REST messaging, wired in factory (27 tests)
- [x] **Tool approval mode** (`src/tools/approval.rs`) — ApprovalGate with policy-based tool gating (24 tests)
- [x] **Agent templates** (`src/config/templates.rs`) — 4 built-in templates + JSON file loading (21 tests)
- [x] **Plugin system** (`src/plugins/`) — JSON manifest plugins with command templates, discovery, validation (70+ tests)

### Deep Wiring (2026-02-14)
- [x] **Tool approval wired** — `ApprovalConfig` on `Config`, `ApprovalGate` checked before each tool execution in agent loop
- [x] **Agent templates wired** — `template list`, `template show` CLI commands, `agent --template <name>` flag, system prompt + config overrides
- [x] **Plugin system wired** — `PluginConfig` on `Config`, `PluginTool` adapter (Tool trait), plugin discovery + registration in `create_agent()`
- [x] **Webhook channel wired** — `WebhookConfig` on `ChannelsConfig`, registered in factory with bind/port/auth/allowlist

### Low Priority (2026-02-14)
- [x] **Telemetry export** (`src/utils/telemetry.rs`) — Prometheus text exposition + JSON renderers, TelemetryConfig (13 tests)
- [x] **Cost tracking** (`src/utils/cost.rs`) — model pricing tables, CostTracker with per-provider/model accumulation, CostConfig (18 tests)
- [x] **Batch mode** (`src/batch.rs`) — load prompts from file (text/jsonl), BatchResult, format output (text/jsonl), CLI command (15+ tests)

### Low Priority Wiring (2026-02-14)
- [x] **Telemetry wired** — `TelemetryConfig` on `Config`, added to KNOWN_TOP_LEVEL in validate.rs
- [x] **Cost tracking wired** — `CostConfig` on `Config`, added to KNOWN_TOP_LEVEL in validate.rs
- [x] **Batch mode wired** — `BatchConfig` on `Config`, `batch` CLI command with --input/--output/--format/--stop-on-error/--stream/--template flags

### Nice-to-Have
- [ ] **Web UI** — browser-based chat interface

## Stats

- Codebase: ~38,000+ lines of Rust
- Tests: ~876 lib + 63 integration + 97 doc = ~1036 total
- Tools: 15 agent tools + plugin tools (dynamic)
- Channels: 4 (Telegram, Slack, Discord, Webhook)
- Providers: 2 (Claude, OpenAI) + RetryProvider + FallbackProvider
