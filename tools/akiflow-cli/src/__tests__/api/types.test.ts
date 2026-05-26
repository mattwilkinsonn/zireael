import { describe, expect, test } from "bun:test";
import type { Task, TaskStatus } from "../../lib/api/types";
import { taskStateOf } from "../../lib/api/types";

function task(overrides: Partial<Task>): Task {
	return {
		id: "t",
		user_id: 1,
		status: 2,
		done: false,
		trashed_at: null,
		deleted_at: null,
		...overrides,
	} as Task;
}

describe("TaskStatus", () => {
	test("widened to 1 | 2 | 9 | 10 (real API values)", () => {
		const values: TaskStatus[] = [1, 2, 9, 10];
		expect(values.length).toBe(4);
	});
});

describe("taskStateOf", () => {
	test("status=1 → 'inbox'", () => {
		expect(taskStateOf(task({ status: 1 }))).toBe("inbox");
	});

	test("status=2 done=false → 'planned'", () => {
		expect(taskStateOf(task({ status: 2, done: false }))).toBe("planned");
	});

	test("status=2 done=true → 'done'", () => {
		expect(taskStateOf(task({ status: 2, done: true }))).toBe("done");
	});

	test("status=10 trashed_at set → 'trashed'", () => {
		expect(
			taskStateOf(task({ status: 10, trashed_at: "2026-05-01T00:00:00Z" })),
		).toBe("trashed");
	});

	test("status=9 deleted_at set → 'deleted'", () => {
		expect(
			taskStateOf(task({ status: 9, deleted_at: "2026-05-01T00:00:00Z" })),
		).toBe("deleted");
	});

	test("deleted_at takes precedence over status", () => {
		expect(
			taskStateOf(
				task({ status: 2, done: true, deleted_at: "2026-05-01T00:00:00Z" }),
			),
		).toBe("deleted");
	});
});
