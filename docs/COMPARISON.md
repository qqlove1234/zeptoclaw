# Comparison: ZeptoClaw vs NanoBot vs NanoClaw

A detailed comparison of three lightweight AI assistant frameworks.

## Overview

| Feature | **ZeptoClaw** | **NanoBot** | **NanoClaw** |
|---------|---------------|-------------|--------------|
| **Language** | Rust | Python | TypeScript |
| **Lines of Code** | ~13,500 | ~8,500 (~3,500 core) | ~5,000 |
| **Philosophy** | Lightweight, secure, self-hosted | Ultra-lightweight, research-ready | Minimal, fork & customize |
| **Repository** | This repo | [HKUDS/nanobot](https://github.com/HKUDS/nanobot) | [qwibitai/nanoclaw](https://github.com/qwibitai/nanoclaw) |

---

## Memory & Persistence

| Feature | **ZeptoClaw** | **NanoBot** | **NanoClaw** |
|---------|---------------|-------------|--------------|
| **Storage** | JSON files (`~/.zeptoclaw/sessions/`) | JSONL sessions + markdown memory files | SQLite database |
| **Session Management** | Per channel:chat_id | Per channel:chat_id (`~/.nanobot/sessions`) + workspace memory files | Per group folder |
| **Long-term Memory** | Full conversation history | Two-layer: facts + searchable log | Per-group `CLAUDE.md` |
| **Cross-session Learning** | No | No | No |
| **Database** | JSON only | Files only | SQLite |
| **Scheduled Tasks** | No | Yes (cron) | Yes |

### Memory Architecture Details

**ZeptoClaw**
```
~/.zeptoclaw/sessions/
├── telegram%3A123456.json    # Full conversation history
├── cli%3Acli.json
└── ...
```
- Two-tier: in-memory HashMap + JSON file persistence
- Auto-save after every message
- Session key format: `channel:chat_id`

**NanoBot**
```
~/.nanobot/workspace/memory/
├── MEMORY.md     # Long-term facts (curated)
└── HISTORY.md    # Grep-searchable conversation log
~/.nanobot/sessions/*.jsonl   # Per channel/chat session history
```
- Mixed persistence: JSONL sessions + human-readable markdown memory
- Agent can read/write memory files
- History append-only log

**NanoClaw**
```
data/store/messages.db (SQLite)
groups/*/CLAUDE.md (per-group memory)
```
- SQLite for messages, tasks, sessions, groups
- Per-group CLAUDE.md for isolated context
- Claude Agent SDK handles session continuity

---

## Channels Supported

| Channel | **ZeptoClaw** | **NanoBot** | **NanoClaw** |
|---------|:-------------:|:-----------:|:------------:|
| CLI | ✅ | ✅ | ❌ |
| Telegram | ✅ | ✅ | Via skill |
| WhatsApp | ❌ | ✅ | ✅ (primary) |
| Discord | ❌ | ✅ | Via skill |
| Slack | ❌ | ✅ | Via skill |
| Feishu | ❌ | ✅ | ❌ |
| DingTalk | ❌ | ✅ | ❌ |
| Email | ❌ | ✅ | ❌ |
| QQ | ❌ | ✅ | ❌ |
| MoChat | ❌ | ✅ | ❌ |

---

## Security & Isolation

| Feature | **ZeptoClaw** | **NanoBot** | **NanoClaw** |
|---------|---------------|-------------|--------------|
| **Sandboxing** | Workspace restriction | `restrictToWorkspace` option | Container isolation |
| **Shell Security** | Regex blocklist patterns | Basic workspace restriction | Container isolation |
| **Path Traversal** | Symlink escape detection | No | Container mounts |
| **Container Runtime** | Native / Docker / Apple Container (for shell execution) | None | Apple Container / Docker |
| **Credential Protection** | Blocked patterns for SSH/AWS/Kube | No | Isolated filesystem |

### Security Model Comparison

**ZeptoClaw** - Application-level security + optional runtime isolation for shell tool
- Regex-based shell command blocklist (prevents `rm -rf /`, reverse shells, etc.)
- Symlink escape detection (prevents workspace breakout via symlinks)
- Path traversal protection with URL-encoded pattern detection
- Credential path blocking (`.ssh/`, `.aws/credentials`, etc.)
- Optional shell execution runtime: native, Docker, or Apple Container

**NanoBot** - Basic workspace restriction
- `restrictToWorkspace: true` sandboxes file operations
- Allowlist-based channel access control
- No shell command filtering

**NanoClaw** - OS-level isolation
- Agents run in Linux containers (Apple Container on macOS, Docker on Linux)
- Each group has isolated filesystem mount
- Shell access safe because commands run inside container
- True process isolation

---

## LLM Providers

| Provider | **ZeptoClaw** | **NanoBot** | **NanoClaw** |
|----------|:-------------:|:-----------:|:------------:|
| Anthropic/Claude | ✅ | ✅ | ✅ (via SDK) |
| OpenAI | ✅ | ✅ | ❌ |
| OpenRouter | ⚠️ (config present; runtime provider not wired) | ✅ | ❌ |
| Local (vLLM) | ⚠️ (config present; runtime provider not wired) | ✅ | ❌ |
| DeepSeek | ❌ | ✅ | ❌ |
| Groq | ⚠️ (config present; runtime provider not wired) | ✅ | ❌ |
| Gemini | ⚠️ (config present; runtime provider not wired) | ✅ | ❌ |
| Moonshot/Kimi | ❌ | ✅ | ❌ |
| Zhipu GLM | ⚠️ (config present; runtime provider not wired) | ✅ | ❌ |
| MiniMax | ❌ | ✅ | ❌ |
| DashScope/Qwen | ❌ | ✅ | ❌ |

---

## Architecture

| Aspect | **ZeptoClaw** | **NanoBot** | **NanoClaw** |
|--------|---------------|-------------|--------------|
| **Runtime** | Tokio async | Python asyncio | Node.js |
| **Message Bus** | Internal pub/sub | Simple routing | Polling loop + queue |
| **Agent Execution** | In-process | In-process | Container per group |
| **Tool System** | Trait-based registry | Skills + tools | Claude Agent SDK |
| **Extensibility** | Code changes | Skills (bundled) | Skills (transform forks) |

### Architecture Diagrams

**ZeptoClaw**
```
Channel (Telegram) → MessageBus → Agent Loop → LLM Provider
                          ↑            ↓
                    SessionManager  ToolRegistry
                          ↓            ↓
                    JSON Files    Shell/Filesystem
```

**NanoBot**
```
Channels (9+) → Bus → Agent Loop → LiteLLM → Providers (10+)
                         ↓
                   MemoryStore
                   (MEMORY.md)
```

**NanoClaw**
```
WhatsApp (baileys) → SQLite → Polling Loop → Container (Claude SDK)
                                    ↓
                              Per-group mount
                             (groups/*/CLAUDE.md)
```

---

## Tools & Capabilities

| Tool | **ZeptoClaw** | **NanoBot** | **NanoClaw** |
|------|:-------------:|:-----------:|:------------:|
| Shell execution | ✅ | ✅ | ✅ (in container) |
| File read/write | ✅ | ✅ | ✅ |
| Web search | ❌ | ✅ (Brave) | ✅ |
| Web fetch | ❌ | ✅ | ✅ |
| Voice transcription | ❌ | ✅ (Groq Whisper) | ❌ |
| Scheduled tasks | ❌ | ✅ | ✅ |
| Agent Swarms | ❌ | ❌ | ✅ |
| Gmail integration | ❌ | ✅ | Via skill |

---

## Performance

| Metric | **ZeptoClaw** | **NanoBot** | **NanoClaw** |
|--------|---------------|-------------|--------------|
| **Binary Size** | ~5MB | N/A (Python) | N/A (Node.js) |
| **Startup Time** | Fast | Medium | Medium |
| **Memory Usage** | Low | Medium | Higher (containers) |
| **Concurrency** | Async (Tokio) | Async (asyncio) | Per-group queue |

---

## Key Differentiators

### ZeptoClaw (Rust)

**Strengths**
- Strongest application-level security (regex shell blocklist, symlink detection)
- Native performance, small binary
- Full conversation persistence across restarts
- Type-safe, memory-safe (Rust)

**Weaknesses**
- Fewer channels (CLI, Telegram only)
- No full agent-level containerization (runtime isolation currently scoped to shell tool execution)
- No scheduled tasks
- Runtime provider wiring narrower than config surface

### NanoBot (Python)

**Strengths**
- Most LLM providers (10+)
- Most channels (9+)
- Voice transcription (Groq Whisper)
- Scheduled tasks with cron
- Easy to extend (Python ecosystem)
- Research-friendly codebase

**Weaknesses**
- No dedicated database layer (uses JSONL + markdown files)
- No container isolation
- Larger dependency footprint

### NanoClaw (TypeScript)

**Strengths**
- True container isolation (Apple Container/Docker)
- Per-group isolated memory (`CLAUDE.md`)
- SQLite database for reliability
- Agent Swarms support
- Skill-based customization (transform forks, not config)
- Smallest codebase

**Weaknesses**
- WhatsApp only (others via skills)
- Claude-only (uses Claude Agent SDK)
- Requires container runtime

---

## Decision Matrix

| If you want... | Choose |
|----------------|--------|
| **Best security + Rust** | ZeptoClaw |
| **Most integrations + Python** | NanoBot |
| **Container isolation + fork customization** | NanoClaw |
| **Research-friendly** | NanoBot |
| **Smallest codebase** | NanoClaw |
| **Multiple LLM providers** | NanoBot |
| **Native binary distribution** | ZeptoClaw |
| **Scheduled/cron tasks** | NanoBot or NanoClaw |
| **Voice messages** | NanoBot |
| **Agent teams/swarms** | NanoClaw |

---

## Quick Start Comparison

### ZeptoClaw
```bash
git clone https://github.com/zeptoclaw/zeptoclaw
cd zeptoclaw
cargo build --release
./target/release/zeptoclaw onboard
./target/release/zeptoclaw agent -m "Hello"
```

### NanoBot
```bash
pip install nanobot-ai
nanobot onboard
nanobot agent -m "Hello"
```

### NanoClaw
```bash
git clone https://github.com/qwibitai/nanoclaw
cd nanoclaw
claude   # Run /setup in Claude Code
```

---

## Conclusion

All three projects share the goal of being lightweight alternatives to larger AI assistant frameworks. Choose based on your priorities:

- **ZeptoClaw**: Security-first, Rust performance, self-hosted simplicity
- **NanoBot**: Maximum integrations, Python flexibility, research-ready
- **NanoClaw**: True isolation via containers, fork-and-customize philosophy

---

## Integration Roadmap for ZeptoClaw

Based on analysis of all three codebases, here are the highest-impact features to integrate into ZeptoClaw:

### Where ZeptoClaw is Currently Better

| Area | Advantage |
|------|-----------|
| **Error Handling** | Cleaner typed Rust core with explicit error handling around provider failures |
| **Shell/Path Security** | Better security baseline than NanoBot (regex blocklist, symlink detection) |
| **Runtime Behavior** | Clear fail-closed behavior with typed runtime abstraction |

### Feature Gap Analysis

| Area | ZeptoClaw | NanoClaw | NanoBot | Gap |
|------|-----------|----------|---------|-----|
| **Runtime Isolation** | Strong typed runtime abstraction + fail-closed fallback | Strong per-group container isolation with mount policy | Less runtime-isolation-centric | Add mount allowlist validation |
| **Providers** | Config supports many, runtime only Anthropic/OpenAI | Claude-focused | Registry-driven multi-provider matching/prefixing | Port provider registry concept |
| **Channels** | Only Telegram implemented/registered | WhatsApp-centric | Broad channel manager | Add channel factory/registry |
| **Scheduling** | No cron service/tool | Built-in scheduled task loop | Full cron service + tool | Add persistent cron service |
| **Background Delegation** | None | Agent swarms via container setup | Explicit subagent manager + spawn tool | Add spawn tool after cron |
| **Tooling Safety** | Strong path+shell guards | Strong mount allowlist hardening | Workspace restriction flag | Add mount policy validation |

### Prioritized Integration Steps

| Priority | Feature | Source | Complexity | Description |
|----------|---------|--------|------------|-------------|
| 1 | **Provider Registry** | NanoBot | Medium | Remove hardcoded provider selection; implement registry-driven resolver abstraction |
| 2 | **Mount Allowlist Validation** | NanoClaw | Low | Validate `runtime.docker.extra_mounts` and `runtime.apple.extra_mounts` before runtime creation |
| 3 | **Cron Service + Tool** | NanoBot/NanoClaw | Medium | Add persistent cron service using `~/.zeptoclaw/` storage |
| 4 | **Channel Registry/Factory** | NanoBot | Medium | Map enabled config to implementations consistently (not just Telegram) |
| 5 | **Subagent Spawn Tool** | NanoBot | High | Bus-based result injection for background task delegation |

### Code References

**ZeptoClaw**
- Runtime abstraction: `src/runtime/factory.rs:15`, `src/config/types.rs:371`
- Provider selection: `src/main.rs:107`, `src/providers/mod.rs:29`
- Channel wiring: `src/channels/mod.rs:111`, `src/main.rs:674`
- Security: `src/security/path.rs:69`, `src/security/shell.rs:122`

**NanoClaw**
- Container runner: `src/container-runner.ts:58`
- Mount security: `src/mount-security.ts:232`
- Task scheduler: `src/task-scheduler.ts:182`

**NanoBot**
- Provider registry: `nanobot/providers/registry.py:63`
- Config schema: `nanobot/config/schema.py:237`
- Channel manager: `nanobot/channels/manager.py:38`
- Cron service: `nanobot/cron/service.py:42`
- Subagent manager: `nanobot/agent/subagent.py:20`
- Spawn tool: `nanobot/agent/tools/spawn.py:11`

---

*Last updated: 2026-02-13*
