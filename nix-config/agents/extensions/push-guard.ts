import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";

// Hard guardrails that must hold even if the model is confused or a prompt
// tries to override them. Commits are allowed; pushes are not (Matt pushes),
// and broad pattern-matching process kills are never allowed.

const PUSH = /\bgit(?:\s+-\S+(?:\s+[^-]\S*)?)*\s+push\b|\bjj(?:\s+-\S+(?:\s+[^-]\S*)?)*\s+git\s+push\b|\b(?:jj-gt|gt)\s+(?:[\w.@/-]+\s+)*submit\b/;
// pkill / killall are always pattern-based -> broad. For `kill`, skip the
// leading signal spec (-9, -KILL, -s NAME, -n NUM, ...) and block only when a
// remaining target is negative (-1 / -<pgid> = a process group / everything);
// a plain `kill <pid>` or `kill -9 <pid>` stays allowed.
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
    if (event.toolName !== "bash") return;
    const cmd = String(event.input.command ?? "");

    if (PUSH.test(cmd)) {
      return {
        block: true,
        reason:
          "Push blocked: Matt pushes/submits, the agent never does (git push / jj git push / jj-gt submit). " +
          "Commit freely; hand the push or submit command to Matt and stop. See rule://commit-conventions.",
      };
    }

    if (hasBroadKill(cmd)) {
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
