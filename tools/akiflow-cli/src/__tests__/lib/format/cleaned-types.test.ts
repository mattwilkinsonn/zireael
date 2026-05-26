import { describe, expect, test } from "bun:test";
import type { Account, Label, Task } from "../../../lib/api/types";
import {
	emptyContext,
	type ResolveContext,
	toCleanedTaskView,
} from "../../../lib/format/cleaned-types";

function ctx(): ResolveContext {
	const c = emptyContext();
	c.labelsById.set("lbl1", { id: "lbl1", title: "Personal" } as Label);
	c.accountsById.set("acc1", {
		id: "acc1",
		identifier: "matt@example.com",
	} as Account);
	return c;
}

function task(o: Partial<Task>): Task {
	return {
		id: "t",
		user_id: 1,
		status: 2,
		done: false,
		trashed_at: null,
		deleted_at: null,
		title: "X",
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
		recurring_id: null,
		recurrence: null,
		connector_id: null,
		origin_id: null,
		akiflow_account_id: null,
		doc: null,
		due_date: null,
		calendar_id: null,
		time_slot_id: null,
		global_created_at: "2026-01-01T00:00:00Z",
		global_updated_at: "2026-01-02T00:00:00Z",
		done_at: null,
		read_at: null,
		...o,
	} as unknown as Task;
}

describe("toCleanedTaskView — status word", () => {
	test("status=2 done=true → 'done'", () => {
		expect(toCleanedTaskView(task({ done: true }), ctx()).status).toBe("done");
	});

	test("status=1 → 'inbox'", () => {
		expect(toCleanedTaskView(task({ status: 1 }), ctx()).status).toBe("inbox");
	});
});

describe("toCleanedTaskView — plan_bucket", () => {
	test("WEEK 21 of 2026 → 2026-W21", () => {
		expect(
			toCleanedTaskView(task({ plan_unit: "WEEK", plan_period: 202621 }), ctx())
				.plan_bucket,
		).toEqual({ unit: "week", period: "2026-W21" });
	});

	test("MONTH 5 of 2026 → 2026-05", () => {
		expect(
			toCleanedTaskView(
				task({ plan_unit: "MONTH", plan_period: 202605 }),
				ctx(),
			).plan_bucket,
		).toEqual({ unit: "month", period: "2026-05" });
	});
});

describe("toCleanedTaskView — project resolution", () => {
	test("listId → project name from labels", () => {
		expect(toCleanedTaskView(task({ listId: "lbl1" }), ctx()).project).toBe(
			"Personal",
		);
	});

	test("unknown listId → null", () => {
		expect(
			toCleanedTaskView(task({ listId: "missing" }), ctx()).project,
		).toBeNull();
	});
});

describe("toCleanedTaskView — source for gmail tasks", () => {
	test("flattens doc fields into source object", () => {
		const t = task({
			connector_id: "gmail",
			origin_id: "thread123",
			akiflow_account_id: "acc1",
			doc: {
				url: "https://mail.google.com/...",
				from: "Sender <s@example.com>",
				subject: "Subject",
				thread_id: "thread123",
			},
		});
		const view = toCleanedTaskView(t, ctx());
		expect(view.source).toEqual({
			connector: "gmail",
			id: "thread123",
			url: "https://mail.google.com/...",
			account: "matt@example.com",
			thread_id: "thread123",
			subject: "Subject",
			from: "Sender <s@example.com>",
		});
	});

	test("non-connector tasks → source: null", () => {
		expect(toCleanedTaskView(task({}), ctx()).source).toBeNull();
	});
});

describe("toCleanedTaskView — overdue", () => {
	test("date in past + not done → overdue: true", () => {
		expect(toCleanedTaskView(task({ date: "2020-01-01" }), ctx()).overdue).toBe(
			true,
		);
	});

	test("date in past + done → overdue: false", () => {
		expect(
			toCleanedTaskView(task({ date: "2020-01-01", done: true }), ctx())
				.overdue,
		).toBe(false);
	});
});
