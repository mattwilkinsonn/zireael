import { expect, test } from "bun:test";

import { evaluate } from "./push-guard";

const bash = (command: string) => evaluate("bash", { command });
const mcp = (tool: string, input: Record<string, unknown>) => evaluate(tool, input);

test("allows feature-branch push / submit under the bot", () => {
	expect(bash("jj-gt submit -b cook-compass-scaffold")).toBeNull();
	expect(bash("jj git push -b hudson-sea-865-aws-provider")).toBeNull();
	expect(bash("git push origin amundsen-fnm-direnv-path")).toBeNull();
	expect(bash("git push --force origin cook-feature-restack")).toBeNull();
});

test("blocks push / force to main", () => {
	expect(bash("git push origin main")?.block).toBe(true);
	expect(bash("git push -f origin main")?.block).toBe(true);
	expect(bash("jj git push -b main")?.block).toBe(true);
	expect(bash("git push origin HEAD:main")?.block).toBe(true);
	expect(bash("git push origin feature:refs/heads/main")?.block).toBe(true);
	expect(bash("git push origin HEAD:refs/heads/main")?.block).toBe(true);
	expect(bash("git push origin refs/heads/main")?.block).toBe(true);
});

test("does not false-positive on 'main' inside a branch name", () => {
	expect(bash("jj-gt submit -b cook-sea-1-main-nav")).toBeNull();
	expect(bash("git push origin feat-main-menu")).toBeNull();
	expect(bash("git push origin main-feature")).toBeNull();
	expect(bash("jj-gt submit -b main-nav")).toBeNull();
});

test("blocks merges — the human gate", () => {
	expect(bash("gh pr merge 123 --squash")?.block).toBe(true);
	expect(bash("jj-gt merge")?.block).toBe(true);
	expect(bash("gh pr -R sealedsecurity/sealed merge 123 --squash")?.block).toBe(true);
});

test("blocks pushes / writes to non-allowlisted owners", () => {
	expect(bash("git push https://github.com/can1357/oh-my-pi main")?.block).toBe(true);
	expect(bash("gh pr create -R can1357/oh-my-pi")?.block).toBe(true);
	expect(bash("gh issue create -R can1357/oh-my-pi -t bug")?.block).toBe(true);
	expect(bash("git push upstream my-branch")?.block).toBe(true);
	expect(bash("gh pr -R can1357/oh-my-pi create")?.block).toBe(true);
});

test("allows pushes / writes to allowlisted owners", () => {
	expect(bash("git push https://github.com/mattwilkinsonn/zireael my-branch")).toBeNull();
	expect(bash("gh pr create -R sealedsecurity/sealed")).toBeNull();
	expect(bash("gh pr view -R can1357/oh-my-pi 42")).toBeNull(); // read of upstream is fine
});

test("github MCP: writes honour the owner allowlist, reads do not", () => {
	expect(mcp("mcp__github_create_pull_request", { owner: "can1357", repo: "oh-my-pi" })?.block).toBe(true);
	expect(mcp("mcp__github_create_pull_request", { owner: "mattwilkinsonn", repo: "zireael" })).toBeNull();
	expect(mcp("mcp__github_pull_request_read", { owner: "can1357", repo: "oh-my-pi" })).toBeNull();
	expect(mcp("mcp__github_create_pull_request", {})?.block).toBe(true); // missing owner → fail closed
	expect(mcp("mcp__github_create_pull_request", { owner: "sealedsecurity", repo: "sealed" })).toBeNull();
});

test("github MCP: merge is always blocked", () => {
	expect(mcp("mcp__github_merge_pull_request", { owner: "sealedsecurity", repo: "sealed" })?.block).toBe(true);
});

test("still blocks broad process kills, allows targeted ones", () => {
	expect(bash("pkill -f node")?.block).toBe(true);
	expect(bash("kill -- -1")?.block).toBe(true);
	expect(bash("kill -9 12345")).toBeNull();
});
