import { describe, expect, test } from "bun:test";
import type { Task } from "../../../lib/api/types";
import { filterTasks } from "../../../lib/filters/task";

function task(overrides: Partial<Task>): Task {
	return {
		id: "t1",
		user_id: 1,
		status: 2,
		done: false,
		trashed_at: null,
		deleted_at: null,
		title: "Task",
		description: null,
		date: null,
		datetime: null,
		datetime_tz: null,
		duration: null,
		priority: null,
		listId: null,
		tags_ids: [],
		plan_unit: null,
		plan_period: null,
		connector_id: null,
		recurring_id: null,
		...overrides,
	} as unknown as Task;
}

describe("filterTasks — status", () => {
	test("default excludes done + trashed", () => {
		const tasks = [
			task({ id: "active", status: 2, done: false }),
			task({ id: "done", status: 2, done: true }),
			task({
				id: "trash",
				status: 10,
				trashed_at: "2026-01-01T00:00:00Z",
			}),
			task({ id: "inbox", status: 1 }),
		];
		expect(
			filterTasks(tasks, {})
				.map((t) => t.id)
				.sort(),
		).toEqual(["active", "inbox"]);
	});

	test("--status done", () => {
		const tasks = [
			task({ id: "a", done: false }),
			task({ id: "b", done: true }),
		];
		expect(filterTasks(tasks, { status: ["done"] }).map((t) => t.id)).toEqual([
			"b",
		]);
	});

	test("--status all", () => {
		const tasks = [
			task({ id: "active", done: false }),
			task({ id: "done", done: true }),
			task({ id: "trash", trashed_at: "2026-01-01T00:00:00Z" }),
			task({ id: "del", status: 9, deleted_at: "2026-01-01T00:00:00Z" }),
		];
		expect(
			filterTasks(tasks, { status: ["all"] })
				.map((t) => t.id)
				.sort(),
		).toEqual(["active", "done", "trash"]);
	});
});

describe("filterTasks — date ranges", () => {
	test("filter by date range matches `date`", () => {
		const tasks = [
			task({ id: "before", date: "2026-05-20" }),
			task({ id: "in", date: "2026-05-21" }),
			task({ id: "after", date: "2026-05-22" }),
		];
		const result = filterTasks(tasks, {
			from: new Date("2026-05-21"),
			to: new Date("2026-05-21"),
		});
		expect(result.map((t) => t.id)).toEqual(["in"]);
	});

	test("date range matches plan_unit=WEEK bucket", () => {
		// Week 21 of 2026 covers around May 18-24
		const tasks = [
			task({ id: "bucket-week-this", plan_unit: "WEEK", plan_period: 202621 }),
			task({ id: "bucket-week-other", plan_unit: "WEEK", plan_period: 202622 }),
		];
		const result = filterTasks(tasks, {
			from: new Date(2026, 4, 21), // May 21 local
			to: new Date(2026, 4, 21),
		});
		expect(result.map((t) => t.id)).toEqual(["bucket-week-this"]);
	});
});

describe("filterTasks — connector / recurring / unplanned", () => {
	test("connector filter", () => {
		const tasks = [
			task({ id: "gmail", connector_id: "gmail" }),
			task({ id: "linear", connector_id: "linear" }),
			task({ id: "native", connector_id: null }),
		];
		expect(filterTasks(tasks, { connector: "gmail" }).map((t) => t.id)).toEqual(
			["gmail"],
		);
		expect(filterTasks(tasks, { connector: "none" }).map((t) => t.id)).toEqual([
			"native",
		]);
	});

	test("recurring filter", () => {
		const tasks = [
			task({ id: "rec", recurring_id: "abc" }),
			task({ id: "one", recurring_id: null }),
		];
		expect(filterTasks(tasks, { recurring: true }).map((t) => t.id)).toEqual([
			"rec",
		]);
	});

	test("unplanned (inbox state)", () => {
		const tasks = [
			task({ id: "inbox", status: 1, date: null, plan_unit: null }),
			task({ id: "planned", status: 2, date: "2026-05-21" }),
			task({
				id: "bucket",
				status: 2,
				plan_unit: "WEEK",
				plan_period: 202621,
			}),
		];
		expect(filterTasks(tasks, { unplanned: true }).map((t) => t.id)).toEqual([
			"inbox",
		]);
	});
});
