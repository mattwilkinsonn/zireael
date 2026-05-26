#!/usr/bin/env bun
import { defineCommand, runMain } from "citty";
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
		version: "0.1.1",
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
