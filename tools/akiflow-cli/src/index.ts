#!/usr/bin/env bun
import { defineCommand, runMain } from "citty";
import pkg from "../package.json" with { type: "json" };
import { add } from "./commands/add";
import { authCommand } from "./commands/auth";
import { block } from "./commands/block";
import { cacheCommand } from "./commands/cache";
import { cal } from "./commands/cal";
import { completionCommand } from "./commands/completion";
import { doCommand } from "./commands/do";
import { doctorCommand } from "./commands/doctor";
import { lsCommand } from "./commands/ls";
import { projectCommand } from "./commands/project";
import { refreshCommand } from "./commands/refresh";
import { taskCommand } from "./commands/task";

const hello = defineCommand({
	meta: {
		name: "hello",
		description: "Say hello",
	},
	run: async () => {
		console.log("Hello from Akiflow CLI!");
	},
});

const main = defineCommand({
	meta: {
		name: "af",
		description: "Akiflow CLI - Task management and automation",
		// Version comes from package.json so `af --version` matches the
		// release the binary was built from. The `scripts/release.sh` release flow
		// bumps `tools/akiflow-cli/package.json:version` in lockstep
		// with the workspace Cargo.toml + Formula/*.rb files, and bun's
		// `--compile` bundles the json import into the binary at build
		// time. Without this, the version was hardcoded and silently
		// drifted from the release tag — homebrew tap formula tests
		// caught it on v0.3.3.
		version: pkg.version,
	},
	subCommands: {
		add,
		hello,
		do: doCommand,
		ls: lsCommand,
		task: taskCommand,
		project: projectCommand,
		completion: completionCommand,
		cal,
		block,
		auth: authCommand,
		cache: cacheCommand,
		doctor: doctorCommand,
		refresh: refreshCommand,
	},
});

runMain(main);
