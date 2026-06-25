import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";

// Hard guardrails that must hold even if the model is confused or a prompt
// tries to override them. Commits are allowed; pushes are not (Matt pushes),
// and broad pattern-matching process kills are never allowed.

const PUSH = /\b(?:git\s+push|jj\s+git\s+push)\b/;
// pkill / killall (any), and kill aimed at a process group / everything
// (`kill -1`, `kill -9 -1`, `kill -- -1`). A plain `kill <pid>` is fine.
const BROAD_KILL = /\b(?:pkill|killall)\b|\bkill\b\s+(?:-\S+\s+)*--?(?:1\b|\s*-1\b)/;

export default function pushGuard(pi: ExtensionAPI): void {
  pi.setLabel("push-guard");

  pi.on("tool_call", async (event) => {
    if (event.toolName !== "bash") return;
    const cmd = String(event.input.command ?? "");

    if (PUSH.test(cmd)) {
      return {
        block: true,
        reason:
          "Push blocked: Matt pushes, the agent never does (git push / jj git push). " +
          "Commit freely; hand the push command to Matt and stop. See rule://commit-conventions.",
      };
    }

    if (BROAD_KILL.test(cmd)) {
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
