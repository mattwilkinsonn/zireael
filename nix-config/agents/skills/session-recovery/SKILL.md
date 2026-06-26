---
name: session-recovery
description: "Inspect or recover a past/broken OMP session from disk: find its transcript + prompt history, parse the JSONL, diagnose a wedge (swallowed provider 400, oversized image payload), and resume from artifacts."
---

# Session Recovery

Everything an OMP session does is on disk. Use this when you need to read a *prior* session's work, or when a session **wedges** — stops producing output, returns empty turns, and stays broken across restart.

`history://<id>` does **not** help here: it only lists in-process subagents of the *current* session. A separate prior session (another process / earlier run) is reachable only through the on-disk stores below.

## Where session state lives

Under `~/.omp/agent/`:

- **Transcripts (full, authoritative):** `sessions/<cwd-slug>/<ISO-ts>_<session-id>.jsonl` — one JSONL file per session; each line is one message or event. `<cwd-slug>` is the working dir with every `/` turned into `-` (e.g. `~/repos/sealedsecurity` → `-repos-sealedsecurity`). **Subagent** transcripts sit in a sibling directory named after the parent file (minus `.jsonl`): `<ISO-ts>_<session-id>/<AgentId>.jsonl`.
- **Prompt history (SQLite):** `history.db`, table `history(id, prompt, created_at, cwd, session_id)` — every user prompt ever typed, with the session it belonged to. Fastest way to find *which* session did *what* and to recover a `session_id`.
- **Blobs:** `blobs/<sha256>` and `blobs/<sha256>.png` — binary payloads (screenshots, compaction frames). Transcripts reference them as `blob:sha256:<hash>` inside an image block's `data`; OMP resolves the ref to base64 only when it builds the provider request.
- **Live state, not transcripts:** `agent.db` (auth, `usage_history`), `config.yml` (settings — e.g. `browser.enabled`, `modelRoles`). Don't go looking for conversation content here.

Under `~/.omp/logs/`:

- **Rejected requests:** `http-400-requests/<ts>-<rand>.json` — the **full outbound request body** (system + messages + tools, including base64 images) for any provider HTTP 400. One file per rejected attempt. This is the fingerprint of a poisoned context.
- **Daily log:** `omp.<date>.log` (JSON lines) — compaction decisions, usage fetches, kernel warnings. Mostly noise; grep narrowly (it does *not* reliably contain provider response bodies).

## Find the session

Recent prompts and their session ids:

```bash
read "~/.omp/agent/history.db?q=SELECT id, substr(prompt,1,80) AS prompt, datetime(created_at,'unixepoch') AS ts, session_id FROM history ORDER BY id DESC LIMIT 25"
```

Map a `session_id` to its transcript file:

```bash
find ~/.omp/agent/sessions -name '*<session-id>*'
```

## Read a transcript without drowning in it

Transcripts are large (tool results are stored inline — tens of KB per line, often many MB total). **Never `read` the raw file whole.** Size it first (`wc -l` / `wc -c`), then parse with an `eval` cell and build an index before reading any range.

Line schema: most lines are `{id, message, parentId, timestamp, type}`. Event lines (no `message`): `session`, `model_change`, `thinking_level_change`, `mcp_tool_selection`, `compaction`, `custom_message`. Inside `message`: `role` ∈ `user` / `assistant` / `toolResult` / `developer` / `fileMention`; `content` is a string or a block list. Block kinds: `text`, `thinking`, a tool call (`name` + `input`), `toolResult` (`content`, `is_error`), `image` (`data` / `source.data` = a `blob:sha256:` ref).

```python
import json
# Stream line-by-line — never materialize a multi-hundred-MB transcript whole.
def blocks_of(m):
    out = []
    c = m.get("content")
    if isinstance(c, list):
        for b in c:
            if not isinstance(b, dict): continue
            if b.get("name"): out.append("call:" + b["name"])             # tool call
            elif b.get("type") == "image": out.append("image")
            else: out.append(b.get("type") or "?")
    return out
with open(P) as f:                                   # P = the .jsonl path
    for i, line in enumerate(f):
        r = json.loads(line)
        m = r.get("message") or {}
        print(i, r.get("type"), m.get("role"), blocks_of(m)[:8])
```

From the index, re-render only the ranges you care about (assistant text + tool-call inputs near the end, the last user turns, the failing tool result). To pull one record verbatim without reloading the whole file, read just its line — `import itertools; rec = json.loads(next(itertools.islice(open(P), i, i+1)))` — then index into `rec["message"]["content"]`.

## Diagnose a wedge

Signature: assistant turns return **empty content**, repeatedly, and the session stays dead after `/quit` + restart. The usual cause is a request the provider **rejects** that OMP then keeps re-sending.

1. Check `~/.omp/logs/http-400-requests/`. Files there (timestamps matching the dead retries) mean the provider 400'd; OMP swallowed it and emitted empty turns. Each file is the request that was rejected.
2. Quantify the payload — an oversized **image** payload is the common culprit:

```python
import json
j = json.load(open(REQ))  # REQ = an http-400-requests/*.json
tot = cnt = 0
def walk(o):
    global tot, cnt
    if isinstance(o, dict):
        t = o.get("type")
        if t == "image":                                  # Anthropic: source.data / data
            cnt += 1; tot += len(o.get("source", {}).get("data", "") or o.get("data", "") or "")
        elif t in ("input_image", "image_url"):           # OpenAI / Responses / Copilot
            iu = o.get("image_url")
            url = iu.get("url", "") if isinstance(iu, dict) else (iu or "")
            cnt += 1; tot += len(url or o.get("data", "") or "")
        for v in o.values(): walk(v)
    elif isinstance(o, list):
        for v in o: walk(v)
walk(j["body"]["messages"])
print(f"{cnt} images, {tot/1e6:.1f} MB base64")
```

Two image sources stack up in `messages[0]` (the post-compaction "resume / HISTORY" block) and never leave it:

- **`snapcompact` compaction** renders archived history into image **frames**. The compaction line's `shortSummary` reads like `Archived N chars of history onto K snapcompact frames` — those K frames become K base64 images carried on *every* later request.
- **Browser screenshots** add more images. Once the cumulative image payload crosses the provider's limit, every request 400s; because the transcript is reloaded on restart, the same poison re-sends → unrecoverable in place.

## Recover / resume

- **Preferred — fresh session + artifacts.** Start a new session and rebuild from durable outputs: files written, the tracker, commits/bookmarks. The transcript tells you *where you stopped*; the files on disk *are* the continuity. Reconstruct the to-do from the last `todo` tool results in the transcript.
- **Surgical un-wedge (advanced, last resort).** With the session's process **stopped**, copy the `.jsonl` aside, then drop the offending image block(s) (or truncate the lines after the poison) and reopen. For a **`snapcompact`** wedge this alone is not enough: the frames are rebuilt from the compaction entry's preserved `snapcompact` data on every context rebuild, so you must also remove or truncate **through that `compaction` line** (or clear its `snapcompact` preserve data) — dropping the image blocks alone leaves the poison to re-render. Risky: a running process may hold the transcript in memory and overwrite your edit — never edit a live session's file.
- **Prevent.** Keep heavy images out of long-lived context: screenshot only when you'll actually read it, set `browser: { enabled: false }` in `config.yml` when not using it, and be wary of image-based compaction on image-heavy sessions.

## File the harness bug too

A swallowed provider error that yields a silent empty turn (no surfaced status/body) and survives restart is an OMP defect, not just a context problem. Capture the evidence (`http-400-requests/*.json`, the transcript lines, the screenshot tool result) and draft a note under `~/notes/oh-my-pi/` for Matt to file.
