# Comparison: replay-cli vs simonw/claude-code-transcripts

Reference: https://github.com/simonw/claude-code-transcripts

## 1. JSONL Line Types Handled

| Line type | simonw | replay-cli | Notes |
|-----------|--------|------------|-------|
| `user` | Yes | Yes | Both extract message content |
| `assistant` | Yes | Yes | Both extract message content |
| `summary` | Yes — first priority for session summary | **No** | simonw checks for `type: "summary"` with a `summary` field. We ignore it entirely. |
| `custom-title` | No (doesn't extract metadata) | Yes | We use it for session title |
| `agent-name` | No | Yes | We use it as fallback title |
| `isCompactSummary` | Yes — marks continuation conversations | **No** | simonw uses this to collapse "session continuation summary" messages in a `<details>` element. We don't track this at all. |
| `isMeta` | Yes — skips meta user messages for summary extraction | **No** | simonw explicitly skips `isMeta` user messages when extracting the first user prompt summary. We take the first user message regardless. |

## 2. Session Discovery / File Filtering

| Aspect | simonw | replay-cli |
|--------|--------|------------|
| Skip agent files | `agent-*` filename prefix | `subagents/` directory |
| Skip tool results | Not mentioned | `tool-results/` directory |
| Skip warmup sessions | Yes — skips sessions where summary is "warmup" or "(no summary)" | **No** |
| Skip `<`-prefixed messages | Yes — skips user messages starting with `<` for summary | **No** |
| Glob pattern | `**/*.jsonl` | Recursive dir walk for `*.jsonl` |
| Project name extraction | Decodes folder names like `-home-user-projects-myapp` → `myapp` | Uses `cwd` field from JSONL |

## 3. Content Block Parsing

| Block type | simonw | replay-cli | Notes |
|------------|--------|------------|-------|
| `text` | Yes | Yes | |
| `thinking` | Yes | Yes | |
| `tool_use` | Yes, with specialized renderers for Write/Edit/Bash/TodoWrite | Yes, generic model | simonw renders each tool type with tailored UX (diff view for Edit, file header for Write, etc.) |
| `tool_result` | Yes, with `is_error` flag, image support, git commit detection | Yes, but flattened to string | **We lose `is_error` and image data.** simonw preserves list-of-blocks content including inline images and error state. |
| `image` | Yes — renders base64 images inline | **No** | simonw handles `type: "image"` blocks with `source.media_type` and `source.data` |
| String content (legacy) | Yes — handles both `string` and `list` content formats | Yes | Both handle `content` as either a plain string or array of blocks |

## 4. Session Metadata

| Field | simonw | replay-cli | Notes |
|-------|--------|------------|-------|
| Session ID | From `sessionId` in JSONL | From `sessionId`, falls back to filename | Same |
| Timestamp | Not extracted as metadata (uses file mtime) | Earliest `timestamp` across all lines | **Different approach.** simonw uses filesystem mtime, we scan all lines for earliest. |
| Model | Not extracted | From first assistant message's `message.model` | We have this, simonw doesn't need it (different purpose) |
| CWD/project path | Not from JSONL (uses folder name) | From `cwd` field | |
| Git branch | Not extracted | From `gitBranch` field | |
| CLI version | Not extracted | From `version` field | |
| Title | Not extracted | From `custom-title` or `agent-name` | |
| File size | Yes (for display) | No | |
| GitHub repo | Auto-detected from `git push` output in tool results, or from API session_context | **No** | Big feature: simonw parses tool results for GitHub URLs |

## 5. Conversation Structure

| Aspect | simonw | replay-cli |
|--------|--------|------------|
| Grouping | Groups messages into "conversations" — each user prompt starts a new conversation (tool results don't) | Flat list of messages |
| Continuations | Detects `isCompactSummary` to mark continuation conversations (collapsed in UI) | Not handled |
| Pagination | 5 prompts per page | N/A (TUI, no pagination) |
| Stop hook filtering | Skips messages starting with `"Stop hook feedback:"` | Not handled |

## 6. Analytics / Stats

| Feature | simonw | replay-cli |
|---------|--------|------------|
| Tool usage counts | Yes — counts per tool, displays summary like "3 bash · 2 edit · 1 write" | **No** |
| Git commit extraction | Yes — regex-parses `[branch hash] message` from tool results | **No** |
| Long text detection | Yes — flags text blocks > 300 chars for index display | **No** |
| Token usage | Not tracked | Yes — `input_tokens`, `output_tokens`, cache tokens |
| Cost calculation | No | No |

## 7. Data Sources

| Source | simonw | replay-cli |
|--------|--------|------------|
| Local JSONL (`~/.claude/projects/`) | Yes | Yes |
| Local JSON (legacy format) | Yes — `data.loglines[]` structure | **No** |
| Web API (Anthropic sessions API) | Yes — fetches via `api.anthropic.com/v1/sessions` using keychain OAuth | **No** |
| Session context from API | Yes — extracts `session_context.outcomes` and `session_context.sources` for git repo info | **No** |

## 8. Output Handling

| Feature | simonw | replay-cli |
|---------|--------|------------|
| Tool result content | Preserves as list-of-blocks (text + images) | Flattens to single string |
| Tool result errors | Preserves `is_error` flag, renders with error styling | **Lost** — `is_error` not captured |
| Image content | Renders inline base64 images | **Dropped** |
| Gist publishing | Yes — uploads to GitHub Gist via `gh` CLI | No |

---

## Key Gaps (Priority Order)

### P0 — Data Loss
1. **`tool_result` content flattening** — We coerce list content to string, losing images and structure. `is_error` flag is dropped.
2. **Image blocks ignored** — `type: "image"` blocks in content are silently dropped by our parser.

### P1 — Parsing Completeness
3. **`summary` line type** — Should be captured; it's a first-class session summary set by Claude Code.
4. **`isMeta` flag** — We may be using meta/system user messages as the session's "first user message" preview, producing garbage summaries.
5. **`isCompactSummary` flag** — Needed to properly identify continuation vs fresh conversations.
6. **Legacy JSON format** — simonw supports the older `{loglines: [...]}` format. Not critical unless we want to support web sessions.

### P2 — Enrichment
7. **Git commit extraction from tool results** — Useful analytics.
8. **Tool usage stats** — Counts of tools used per session.
9. **Skip warmup/empty sessions** — Prevent noise in session lists.
10. **Skip `<`-prefixed and stop-hook messages** — Cleaner summaries.

### P3 — Future Capability
11. **Web API session fetching** — simonw can pull sessions from Anthropic's API using keychain credentials. Different use case but interesting.
12. **GitHub repo detection** — Parsing git push output to auto-link commits.

---

## Plan

### Phase 1: Fix Data Loss (P0)

**1a. Enrich `ContentBlock::ToolResult`**
- Add `is_error: bool` field
- Change `content` from `String` to an enum or `serde_json::Value` that preserves list-of-blocks (text + image)
- Parse `is_error` from JSONL

**1b. Add `ContentBlock::Image`**
- New variant: `Image { media_type: String, data: String }` (base64)
- Parse `type: "image"` blocks in `parse_content_blocks()`

### Phase 2: Parsing Completeness (P1)

**2a. Handle `summary` line type**
- During discovery, check for `type: "summary"` lines and use `summary` field as session summary (higher priority than first user message)

**2b. Handle `isMeta` flag**
- Skip `isMeta: true` user messages when extracting `first_user_message`
- Skip user messages where content starts with `<`

**2c. Handle `isCompactSummary`**
- Add `is_continuation: bool` to `Message` or a new field on the session
- During `load_messages()`, carry this flag through

### Phase 3: Session Quality (P2)

**3a. Skip warmup/empty sessions**
- During discovery, skip sessions where the only user message is "warmup" or where no meaningful user message is found

**3b. Tool usage analytics**
- After loading messages, compute tool usage counts per session
- Expose as session metadata for the server

**3c. Git commit extraction**
- Regex-parse tool result content for `[branch hash] message` patterns
- Attach as session metadata
