import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";

// Hard guardrails that must hold even if the model is confused or a prompt
// tries to override them. Commits are allowed; pushes are not (Matt pushes),
// and broad pattern-matching process kills are never allowed.

const PUSH = /\bgit(?:\s+-\S+(?:\s+[^-]\S*)?)*\s+push\b|\bjj(?:\s+-\S+(?:\s+[^-]\S*)?)*\s+git\s+push\b|\b(?:jj-gt|gt)\s+(?:[\w.@/-]+\s+)*(?:submit|ss?)\b/;
// OMP runs commands through several tools; guard all of them for pushes — not
// just bash — so a push/submit can't slip through `ssh` or a `recipe`. The
// broad-kill check stays scoped to locally-executing tools (bash, recipe): a
// kill issued over ssh runs on the remote, not the session's own runtime.
const PUSH_TOOLS = new Set(["bash", "ssh", "recipe"]);
const LOCAL_TOOLS = new Set(["bash", "recipe"]);
// pkill / killall are always pattern-based -> broad. For `kill`, skip the
// leading signal spec (-9, -KILL, -s NAME, -n NUM, ...) and block when a
// remaining TARGET is negative (-1 / -<pgid> = a process group / everything).
// `kill -1 <pid>` (SIGHUP to one PID) and `kill -9 <pid>` stay allowed; only a
// negative target like `kill -- -1` or `kill -TERM -1` is the broad form.
function hasBroadKill(cmd: string): boolean {
  for (const seg of cmd.split(/[\n;&|]+/)) {
    const toks = seg.trim().split(/\s+/).filter(Boolean);
    const idx = toks.findIndex((t) => /(?:^|\/)(?:pkill|killall|kill)$/.test(t));
    if (idx === -1) continue;
    if (!/(?:^|\/)kill$/.test(toks[idx])) return true; // pkill / killall
    let k = idx + 1;
    if (toks[k] === "-s" || toks[k] === "-n") k += 2;
    else if (k < toks.length && /^-[A-Za-z0-9]+$/.test(toks[k])) k += 1;
    if (toks[k] === "--") k += 1;
    if (toks.slice(k).some((t) => /^-\d+$/.test(t))) return true; // negative target
  }
  return false;
}

export default function pushGuard(pi: ExtensionAPI): void {
  pi.setLabel("push-guard");

  pi.on("tool_call", async (event) => {
    if (!PUSH_TOOLS.has(event.toolName)) return;
    // bash/ssh carry the command in `command`; recipe (or any other shape)
    // falls back to the whole input so a push/submit in any field is caught.
    const inp = (event.input ?? {}) as Record<string, unknown>;
    const cmd = typeof inp.command === "string" ? inp.command : JSON.stringify(inp);

    if (PUSH.test(cmd)) {
      return {
        block: true,
        reason:
          "Push blocked: Matt pushes/submits, the agent never does (git push / jj git push / jj-gt submit). " +
          "Commit freely; hand the push or submit command to Matt and stop. See rule://commit-conventions.",
      };
    }

    if (LOCAL_TOOLS.has(event.toolName) && hasBroadKill(cmd)) {
      return {
        block: true,
        reason:
          "Broad process kill blocked (pkill / killall / kill -1). These can take down " +
          "the session's own runtime or unrelated work. Kill a specific PID you started, " +
          "or ask. See rule://process-safety.",
      };
    }
  });
}
