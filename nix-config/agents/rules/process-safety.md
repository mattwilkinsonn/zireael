---
description: "Never use broad pattern-matching process kills; kill only PIDs you started, else ask."
alwaysApply: true
---

# Process Safety

Never use broad `pkill` / `killall` / `kill -9 -1` or any pattern-matching process kill. They can take down the session's own runtime, sibling sessions, orphan test daemons, or unrelated work that happens to match.

To kill something, you MUST know its specific PID **and** know you started it this turn:

- A test fixture (`TestDaemon`-style) or PID file in the test's tempdir tells you the exact PID — kill that.
- Otherwise use `kill <explicit-pid>` only when you can name the PID you launched.
- If you cannot identify the specific PID, ASK before killing anything.

Why the "macOS is safe" intuition is wrong:

- BSD `pkill` / `pgrep` skips the caller's own ancestors by default — but *sibling* processes matching the pattern still die.
- Linux has no ancestor exclusion: the same command can kill the session outright.
- `kill <explicit-pid>` has no ancestor protection on any platform.
- A future sandbox or PID-namespace change could remove the BSD behavior entirely.

`pgrep -af <pattern>` (with `-a`) is for **observing** what would match while diagnosing — never as a prelude to `pkill`. Any long-running process you did not spawn this turn is off-limits: describe what you see and ask before reaching for `kill`.
