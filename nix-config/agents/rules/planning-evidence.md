---
description: "Plan/design docs must pair every claim about external code with file+line and a quoted snippet from this session; no source, no claim."
---

# Planning: Evidence-Paired Cross-Checks

When writing a plan doc, design doc, or any artifact that references code you do not own — another repo, a library, a reference implementation — every claim about that code MUST be paired with evidence gathered in the current session.

## What every claim needs

- **File + line reference** for any claim about what external code does. Point at the specific location: `foo.ts:42` or `bar.rs:100-115`.
- **Quoted snippet** for any claim about specific behavior, field names, string literals, or defaults. Paste the actual lines, not a paraphrase.
- **No directional history claims without the artifact pasted.** "They migrated from X to Y", "the project converged on Z", "tool W deprecated Q" — these are the easiest claims to fabricate because history is rarely checked. Don't make them unless the commit, PR, or changelog entry is in front of you and quoted.
- **No "cross-checked against X" header without quoted lines under it.** That phrase is a factual claim about work done this session; it is only true when the evidence is visible directly beneath it.
- **Code blocks labeled as matching external behavior need the external code quoted next to them.** A proposed implementation labeled "matches the upstream handler" without the upstream source pasted is speculative design, not a cross-check.

## When a read fails

If a file read failed or was denied, the plan does not get to claim it read the file. Write "I couldn't read X; this section is unverified" and stop. A plan with acknowledged gaps is recoverable; a plan full of fabrications presented as verified is a trap.

## The tell

Plan docs get scanned for these patterns. Confident prose is not a signal of grounding — the **absence of pasted source is the tell**. When in doubt, paste the lines or mark the section unverified.
