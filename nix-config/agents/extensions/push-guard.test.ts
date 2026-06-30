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

test("blocks a bare gh issue/pr create with no allowlisted target", () => {
	expect(bash("gh issue create --title bug --body repro")?.block).toBe(true);
	expect(bash("gh pr create --fill")?.block).toBe(true);
	expect(bash("cd /tmp/upstream && gh issue create -t spam")?.block).toBe(true);
	expect(bash("gh issue new --title bug")?.block).toBe(true); // `new` is a create alias
	expect(bash("env GH_TOKEN=x gh issue create -t bug")?.block).toBe(true); // behind a wrapper
	expect(bash("timeout 30 gh pr create --fill")?.block).toBe(true); // behind a wrapper
	expect(bash("gh issue create --body https://github.com/mattwilkinsonn/zireael")?.block).toBe(true); // URL in body is not a target
});

test("allows gh create with an allowlisted -R, and non-create reads", () => {
	expect(bash("gh issue create -R mattwilkinsonn/zireael -t bug")).toBeNull();
	expect(bash("gh pr create -R sealedsecurity/sealed --fill")).toBeNull();
	expect(bash('gh pr create -R "sealedsecurity/sealed" --fill')).toBeNull(); // quoted target
	expect(bash("gh issue create --repo github.com/mattwilkinsonn/zireael -t bug")).toBeNull(); // host-qualified
	expect(bash("gh issue list --label create")).toBeNull(); // "create" is a flag value, not the verb
	expect(bash("gh pr comment 123 --body create")).toBeNull();
	expect(bash("gh issue list")).toBeNull();
	expect(bash('git commit -m "note: block bare gh issue create"')).toBeNull(); // gh only in a message
	expect(bash('jj describe -m "gh pr create guard"')).toBeNull();
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

test("blocks push to main on any remote, not just origin", () => {
	expect(bash("git push fork main")?.block).toBe(true);
	expect(bash("git push -f myremote main")?.block).toBe(true);
	expect(bash("git push fork main-feature")).toBeNull(); // a feature branch, not main
});

test("blocks jj-gt submit --merge-when-ready / -m (bypasses the merge gate)", () => {
	expect(bash("jj-gt submit -b foo --merge-when-ready")?.block).toBe(true);
	expect(bash("jj-gt submit -b foo -m")?.block).toBe(true);
	expect(bash("gt submit --merge-when-ready")?.block).toBe(true);
	expect(bash("jj-gt submit -b foo")).toBeNull(); // a normal submit is fine
	expect(bash("jj-gt submit -b foo --no-hooks")).toBeNull();
});

test("blocks gh api writes to non-allowlisted owners, allows reads + allowlisted", () => {
	expect(bash("gh api repos/can1357/oh-my-pi/issues -f title=x")?.block).toBe(true);
	expect(bash("gh api -X POST repos/can1357/oh-my-pi/pulls")?.block).toBe(true);
	expect(bash("gh api repos/can1357/oh-my-pi/issues")).toBeNull(); // GET read is fine
	expect(bash("gh api repos/mattwilkinsonn/zireael/issues -f title=x")).toBeNull(); // allowlisted
});

test("sees gh through a direnv exec wrapper", () => {
	expect(bash("direnv exec /home/x gh issue create -t bug")?.block).toBe(true); // bare create
	expect(bash("direnv exec /repo gh pr create -R can1357/oh-my-pi")?.block).toBe(true);
	expect(bash("direnv exec /repo gh pr view -R can1357/oh-my-pi 42")).toBeNull(); // read is fine
	expect(bash("direnv exec /repo gh issue create -R mattwilkinsonn/zireael -t bug")).toBeNull();
});

test("a --repo inside a flag value is not a real target", () => {
	expect(bash('gh issue create --body "--repo mattwilkinsonn/zireael"')?.block).toBe(true);
	expect(bash("gh issue create --repo mattwilkinsonn/zireael -t bug")).toBeNull(); // a real -R
});

test("does not false-positive on a later checkout main in a compound command", () => {
	expect(bash("git push origin feature && git checkout main")).toBeNull();
	expect(bash("git push origin feature && echo main")).toBeNull();
	expect(bash("git push fork main && echo done")?.block).toBe(true); // still blocks a real main push
});

test("blocks merge-when-ready on the gt ss / s submit aliases", () => {
	expect(bash("gt ss -m")?.block).toBe(true);
	expect(bash("gt ss --merge-when-ready")?.block).toBe(true);
	expect(bash("gt s --merge-when-ready")?.block).toBe(true);
});

test("gh api writes: fail-closed on an unparsed owner, exempt explicit GET", () => {
	expect(bash("gh api repos/{owner}/{repo}/issues -f title=x")?.block).toBe(true); // placeholder owner
	expect(bash("gh api -X GET repos/can1357/oh-my-pi/issues -f per_page=100")).toBeNull(); // explicit read
	expect(bash("gh api repos/mattwilkinsonn/zireael/issues -f title=x")).toBeNull(); // allowlisted write
});
