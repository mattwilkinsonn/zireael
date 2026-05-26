/// <reference types="bun" />
import { afterEach, beforeEach, describe, expect, it, spyOn } from "bun:test";
import * as fs from "node:fs/promises";
import { getTaskDisplayTitle, lsCommand } from "../../commands/ls";
import { AkiflowClient } from "../../lib/api/client";
import type { Task } from "../../lib/api/types";

let readFileMock: ReturnType<typeof spyOn> | null = null;

beforeEach(() => {
	// Prevent tests from reading the real ~/.cache/af tasks cache
	// (which can make tests non-deterministic and hit the real API).
	readFileMock = spyOn(fs, "readFile").mockRejectedValue(new Error("no cache"));
});

afterEach(() => {
	readFileMock?.mockRestore();
	readFileMock = null;
});

const today = new Date();
const todayDateString = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")}`;
const tomorrow = new Date(today);
tomorrow.setDate(tomorrow.getDate() + 1);
const tomorrowDateString = `${tomorrow.getFullYear()}-${String(tomorrow.getMonth() + 1).padStart(2, "0")}-${String(tomorrow.getDate()).padStart(2, "0")}`;

const weekdayMap = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"] as const;
const todayByday = weekdayMap[today.getDay()]!;

const mockTasks: Task[] = [
	{
		id: "task-1",
		user_id: 1,
		recurring_id: null,
		title: "Complete project documentation",
		description: null,
		date: todayDateString,
		datetime: null,
		datetime_tz: null,
		original_date: null,
		original_datetime: null,
		duration: null,
		recurrence: null,
		recurrence_version: null,
		status: 0,
		priority: 2,
		dailyGoal: null,
		done: false,
		done_at: null,
		read_at: null,
		listId: "list-1",
		section_id: null,
		tags_ids: [],
		sorting: 0,
		sorting_label: null,
		origin: null,
		due_date: null,
		connector_id: null,
		origin_id: null,
		origin_account_id: null,
		akiflow_account_id: null,
		doc: {},
		calendar_id: null,
		time_slot_id: null,
		links: [],
		content: {},
		trashed_at: null,
		plan_unit: null,
		plan_period: null,
		global_list_id_updated_at: null,
		global_tags_ids_updated_at: null,
		global_created_at: "2024-01-01T00:00:00Z",
		global_updated_at: "2024-01-01T00:00:00Z",
		data: {},
		deleted_at: null,
	},
	{
		id: "task-2",
		user_id: 1,
		recurring_id: null,
		title: "Inbox task without date",
		description: null,
		date: null,
		datetime: null,
		datetime_tz: null,
		original_date: null,
		original_datetime: null,
		duration: null,
		recurrence: null,
		recurrence_version: null,
		status: 0,
		priority: 1,
		dailyGoal: null,
		done: false,
		done_at: null,
		read_at: null,
		listId: "list-2",
		section_id: null,
		tags_ids: [],
		sorting: 1,
		sorting_label: null,
		origin: null,
		due_date: null,
		connector_id: null,
		origin_id: null,
		origin_account_id: null,
		akiflow_account_id: null,
		doc: {},
		calendar_id: null,
		time_slot_id: null,
		links: [],
		content: {},
		trashed_at: null,
		plan_unit: null,
		plan_period: null,
		global_list_id_updated_at: null,
		global_tags_ids_updated_at: null,
		global_created_at: "2024-01-01T00:00:00Z",
		global_updated_at: "2024-01-01T00:00:00Z",
		data: {},
		deleted_at: null,
	},
	{
		id: "task-3",
		user_id: 1,
		recurring_id: null,
		title: "Completed task",
		description: null,
		date: todayDateString,
		datetime: null,
		datetime_tz: null,
		original_date: null,
		original_datetime: null,
		duration: null,
		recurrence: null,
		recurrence_version: null,
		status: 2,
		priority: 1,
		dailyGoal: null,
		done: true,
		done_at: "2024-01-01T10:00:00Z",
		read_at: null,
		listId: "list-1",
		section_id: null,
		tags_ids: [],
		sorting: 2,
		sorting_label: null,
		origin: null,
		due_date: null,
		connector_id: null,
		origin_id: null,
		origin_account_id: null,
		akiflow_account_id: null,
		doc: {},
		calendar_id: null,
		time_slot_id: null,
		links: [],
		content: {},
		trashed_at: null,
		plan_unit: null,
		plan_period: null,
		global_list_id_updated_at: null,
		global_tags_ids_updated_at: null,
		global_created_at: "2024-01-01T00:00:00Z",
		global_updated_at: "2024-01-01T10:00:00Z",
		data: {},
		deleted_at: null,
	},
	{
		id: "task-4",
		user_id: 1,
		recurring_id: null,
		title: "Tomorrow task",
		description: null,
		date: tomorrowDateString,
		datetime: null,
		datetime_tz: null,
		original_date: null,
		original_datetime: null,
		duration: null,
		recurrence: null,
		recurrence_version: null,
		status: 0,
		priority: 1,
		dailyGoal: null,
		done: false,
		done_at: null,
		read_at: null,
		listId: "list-2",
		section_id: null,
		tags_ids: [],
		sorting: 3,
		sorting_label: null,
		origin: null,
		due_date: null,
		connector_id: null,
		origin_id: null,
		origin_account_id: null,
		akiflow_account_id: null,
		doc: {},
		calendar_id: null,
		time_slot_id: null,
		links: [],
		content: {},
		trashed_at: null,
		plan_unit: null,
		plan_period: null,
		global_list_id_updated_at: null,
		global_tags_ids_updated_at: null,
		global_created_at: "2024-01-01T00:00:00Z",
		global_updated_at: "2024-01-01T00:00:00Z",
		data: {},
		deleted_at: null,
	},
	{
		id: "task-5",
		user_id: 1,
		recurring_id: null,
		title: "Work project task",
		description: null,
		date: todayDateString,
		datetime: null,
		datetime_tz: null,
		original_date: null,
		original_datetime: null,
		duration: null,
		recurrence: null,
		recurrence_version: null,
		status: 0,
		priority: 1,
		dailyGoal: null,
		done: false,
		done_at: null,
		read_at: null,
		listId: "work-project-1",
		section_id: null,
		tags_ids: [],
		sorting: 4,
		sorting_label: null,
		origin: null,
		due_date: null,
		connector_id: null,
		origin_id: null,
		origin_account_id: null,
		akiflow_account_id: null,
		doc: {},
		calendar_id: null,
		time_slot_id: null,
		links: [],
		content: {},
		trashed_at: null,
		plan_unit: null,
		plan_period: null,
		global_list_id_updated_at: null,
		global_tags_ids_updated_at: null,
		global_created_at: "2024-01-01T00:00:00Z",
		global_updated_at: "2024-01-01T00:00:00Z",
		data: {},
		deleted_at: null,
	},
];

describe("ls command", () => {
	it("shows today's tasks by default", async () => {
		const getTasksMock = spyOn(
			AkiflowClient.prototype,
			"getTasks",
		).mockResolvedValue({
			success: true,
			message: null,
			data: mockTasks,
		});

		const mkdirMock = spyOn(fs, "mkdir").mockResolvedValue(undefined);
		const writeFileMock = spyOn(fs, "writeFile").mockResolvedValue(undefined);

		const consoleLogSpy = spyOn(console, "log").mockImplementation(() => {});

		await lsCommand.run!({
			args: {
				inbox: false,
				all: false,
				done: false,
				json: false,
				plain: false,
				_task: [],
			},
		} as never);

		expect(getTasksMock).toHaveBeenCalled();
		const consoleOutput = consoleLogSpy.mock.calls[0]?.[0] as
			| string
			| undefined;
		expect(consoleOutput).toContain("Complete project documentation");
		expect(consoleOutput).not.toContain("Inbox task without date");
		expect(consoleOutput).not.toContain("Tomorrow task");
		expect(consoleOutput).not.toContain("Completed task");

		consoleLogSpy.mockRestore();
		getTasksMock.mockRestore();
		mkdirMock.mockRestore();
		writeFileMock.mockRestore();
	});

	it("shows virtual recurring tasks for today when Akiflow API has no pending instance", async () => {
		const recurringMaster: Task = {
			...mockTasks[0]!,
			id: "recurring-master-1",
			title: "General 작성",
			date: null,
			recurring_id: null,
			recurrence: `RRULE:FREQ=WEEKLY;BYDAY=${todayByday}`,
			recurrence_version: 1,
		};

		const getTasksMock = spyOn(
			AkiflowClient.prototype,
			"getTasks",
		).mockResolvedValue({
			success: true,
			message: null,
			data: [recurringMaster],
		});

		const mkdirMock = spyOn(fs, "mkdir").mockResolvedValue(undefined);
		const writeFileMock = spyOn(fs, "writeFile").mockResolvedValue(undefined);

		const consoleLogSpy = spyOn(console, "log").mockImplementation(() => {});

		await lsCommand.run!({
			args: {
				inbox: false,
				all: false,
				done: false,
				json: false,
				plain: false,
				_task: [],
			},
		} as never);

		const consoleOutput = consoleLogSpy.mock.calls[0]?.[0] as
			| string
			| undefined;
		expect(consoleOutput).toContain("General 작성");

		consoleLogSpy.mockRestore();
		getTasksMock.mockRestore();
		mkdirMock.mockRestore();
		writeFileMock.mockRestore();
	});

	it("does not create a virtual recurring task when today's instance is already done", async () => {
		const recurringMaster: Task = {
			...mockTasks[0]!,
			id: "recurring-master-2",
			title: "General 작성",
			date: null,
			recurring_id: null,
			recurrence: `RRULE:FREQ=WEEKLY;BYDAY=${todayByday}`,
			recurrence_version: 1,
		};

		const doneInstance: Task = {
			...mockTasks[0]!,
			id: "recurring-instance-done-1",
			title: "General 작성",
			recurring_id: recurringMaster.id,
			date: todayDateString,
			done: true,
			status: 2,
			done_at: "2024-01-01T10:00:00Z",
			recurrence: null,
			recurrence_version: null,
			global_updated_at: "2024-01-01T10:00:00Z",
		};

		const getTasksMock = spyOn(
			AkiflowClient.prototype,
			"getTasks",
		).mockResolvedValue({
			success: true,
			message: null,
			data: [recurringMaster, doneInstance],
		});

		const mkdirMock = spyOn(fs, "mkdir").mockResolvedValue(undefined);
		const writeFileMock = spyOn(fs, "writeFile").mockResolvedValue(undefined);

		const consoleLogSpy = spyOn(console, "log").mockImplementation(() => {});

		await lsCommand.run!({
			args: {
				inbox: false,
				all: false,
				done: false,
				json: false,
				plain: false,
				_task: [],
			},
		} as never);

		const consoleOutput = consoleLogSpy.mock.calls[0]?.[0] as
			| string
			| undefined;
		expect(consoleOutput).not.toContain("General 작성");

		consoleLogSpy.mockRestore();
		getTasksMock.mockRestore();
		mkdirMock.mockRestore();
		writeFileMock.mockRestore();
	});

	it("shows inbox tasks with --inbox flag", async () => {
		const getTasksMock = spyOn(
			AkiflowClient.prototype,
			"getTasks",
		).mockResolvedValue({
			success: true,
			message: null,
			data: mockTasks,
		});

		const mkdirMock = spyOn(fs, "mkdir").mockResolvedValue(undefined);
		const writeFileMock = spyOn(fs, "writeFile").mockResolvedValue(undefined);

		const consoleLogSpy = spyOn(console, "log").mockImplementation(() => {});

		await lsCommand.run!({
			args: {
				inbox: true,
				all: false,
				done: false,
				json: false,
				plain: false,
				_task: [],
			},
		} as never);

		const consoleOutput = consoleLogSpy.mock.calls[0]?.[0] as
			| string
			| undefined;
		expect(consoleOutput).toContain("Inbox task without date");
		expect(consoleOutput).not.toContain("Complete project documentation");
		expect(consoleOutput).not.toContain("Tomorrow task");

		consoleLogSpy.mockRestore();
		getTasksMock.mockRestore();
		mkdirMock.mockRestore();
		writeFileMock.mockRestore();
	});

	it("shows all tasks with --all flag", async () => {
		const getTasksMock = spyOn(
			AkiflowClient.prototype,
			"getTasks",
		).mockResolvedValue({
			success: true,
			message: null,
			data: mockTasks,
		});

		const mkdirMock = spyOn(fs, "mkdir").mockResolvedValue(undefined);
		const writeFileMock = spyOn(fs, "writeFile").mockResolvedValue(undefined);

		const consoleLogSpy = spyOn(console, "log").mockImplementation(() => {});

		await lsCommand.run!({
			args: {
				inbox: false,
				all: true,
				done: false,
				json: false,
				plain: false,
				_task: [],
			},
		} as never);

		const consoleOutput = consoleLogSpy.mock.calls[0]?.[0] as
			| string
			| undefined;
		expect(consoleOutput).toContain("Complete project documentation");
		expect(consoleOutput).toContain("Inbox task without date");
		expect(consoleOutput).toContain("Tomorrow task");

		consoleLogSpy.mockRestore();
		getTasksMock.mockRestore();
		mkdirMock.mockRestore();
		writeFileMock.mockRestore();
	});

	it("shows only completed tasks with --done flag", async () => {
		const getTasksMock = spyOn(
			AkiflowClient.prototype,
			"getTasks",
		).mockResolvedValue({
			success: true,
			message: null,
			data: mockTasks,
		});

		const mkdirMock = spyOn(fs, "mkdir").mockResolvedValue(undefined);
		const writeFileMock = spyOn(fs, "writeFile").mockResolvedValue(undefined);

		const consoleLogSpy = spyOn(console, "log").mockImplementation(() => {});

		await lsCommand.run!({
			args: {
				inbox: false,
				all: false,
				done: true,
				json: false,
				plain: false,
				_task: [],
			},
		} as never);

		const consoleOutput = consoleLogSpy.mock.calls[0]?.[0] as
			| string
			| undefined;
		expect(consoleOutput).toContain("Completed task");
		expect(consoleOutput).not.toContain("Complete project documentation");
		expect(consoleOutput).not.toContain("Inbox task without date");

		consoleLogSpy.mockRestore();
		getTasksMock.mockRestore();
		mkdirMock.mockRestore();
		writeFileMock.mockRestore();
	});

	it("filters tasks by project with --project flag", async () => {
		const getTasksMock = spyOn(
			AkiflowClient.prototype,
			"getTasks",
		).mockResolvedValue({
			success: true,
			message: null,
			data: mockTasks,
		});

		const mkdirMock = spyOn(fs, "mkdir").mockResolvedValue(undefined);
		const writeFileMock = spyOn(fs, "writeFile").mockResolvedValue(undefined);

		const consoleLogSpy = spyOn(console, "log").mockImplementation(() => {});

		await lsCommand.run!({
			args: {
				inbox: false,
				all: false,
				done: false,
				project: "work",
				json: false,
				plain: false,
				_task: [],
			},
		} as never);

		const consoleOutput = consoleLogSpy.mock.calls[0]?.[0] as
			| string
			| undefined;
		expect(consoleOutput).toContain("Work project task");
		expect(consoleOutput).not.toContain("Complete project documentation");

		consoleLogSpy.mockRestore();
		getTasksMock.mockRestore();
		mkdirMock.mockRestore();
		writeFileMock.mockRestore();
	});

	it("outputs JSON format with --json flag", async () => {
		const getTasksMock = spyOn(
			AkiflowClient.prototype,
			"getTasks",
		).mockResolvedValue({
			success: true,
			message: null,
			data: mockTasks,
		});

		const mkdirMock = spyOn(fs, "mkdir").mockResolvedValue(undefined);
		const writeFileMock = spyOn(fs, "writeFile").mockResolvedValue(undefined);

		const consoleLogSpy = spyOn(console, "log").mockImplementation(() => {});

		await lsCommand.run!({
			args: {
				inbox: false,
				all: true,
				done: false,
				json: true,
				plain: false,
				_task: [],
			},
		} as never);

		const consoleOutput = consoleLogSpy.mock.calls[0]?.[0] as
			| string
			| undefined;
		if (consoleOutput) {
			const parsed = JSON.parse(consoleOutput);
			expect(Array.isArray(parsed)).toBe(true);
			expect(parsed).toHaveLength(5);
			expect(parsed[0].id).toBe("task-1");
		}

		consoleLogSpy.mockRestore();
		getTasksMock.mockRestore();
		mkdirMock.mockRestore();
		writeFileMock.mockRestore();
	});

	it("outputs plain text without colors with --plain flag", async () => {
		const getTasksMock = spyOn(
			AkiflowClient.prototype,
			"getTasks",
		).mockResolvedValue({
			success: true,
			message: null,
			data: mockTasks,
		});

		const mkdirMock = spyOn(fs, "mkdir").mockResolvedValue(undefined);
		const writeFileMock = spyOn(fs, "writeFile").mockResolvedValue(undefined);

		const consoleLogSpy = spyOn(console, "log").mockImplementation(() => {});

		await lsCommand.run!({
			args: {
				inbox: false,
				all: false,
				done: false,
				json: false,
				plain: true,
				_task: [],
			},
		} as never);

		const consoleOutput = consoleLogSpy.mock.calls[0]?.[0] as
			| string
			| undefined;
		expect(consoleOutput).toContain("Complete project documentation");
		expect(consoleOutput).not.toContain("\x1b[");

		consoleLogSpy.mockRestore();
		getTasksMock.mockRestore();
		mkdirMock.mockRestore();
		writeFileMock.mockRestore();
	});

	it("shows 'No tasks found' when no tasks match filter", async () => {
		const getTasksMock = spyOn(
			AkiflowClient.prototype,
			"getTasks",
		).mockResolvedValue({
			success: true,
			message: null,
			data: [],
		});

		const mkdirMock = spyOn(fs, "mkdir").mockResolvedValue(undefined);
		const writeFileMock = spyOn(fs, "writeFile").mockResolvedValue(undefined);

		const consoleLogSpy = spyOn(console, "log").mockImplementation(() => {});

		await lsCommand.run!({
			args: {
				inbox: false,
				all: false,
				done: false,
				json: false,
				plain: false,
				_task: [],
			},
		} as never);

		const consoleOutput = consoleLogSpy.mock.calls[0]?.[0] as
			| string
			| undefined;
		expect(consoleOutput).toContain("No tasks found");

		consoleLogSpy.mockRestore();
		getTasksMock.mockRestore();
		mkdirMock.mockRestore();
		writeFileMock.mockRestore();
	});

	it("saves task context to file for short ID resolution", async () => {
		const getTasksMock = spyOn(
			AkiflowClient.prototype,
			"getTasks",
		).mockResolvedValue({
			success: true,
			message: null,
			data: mockTasks,
		});

		const mkdirMock = spyOn(fs, "mkdir").mockResolvedValue(undefined);

		let writtenData = "";
		const writeFileMock = spyOn(fs, "writeFile").mockImplementation((async (
			_file: unknown,
			data: unknown,
		) => {
			writtenData = String(data);
			return undefined;
			// biome-ignore lint/suspicious/noExplicitAny: matching fs.writeFile's broad overload set
		}) as any);

		const consoleLogSpy = spyOn(console, "log").mockImplementation(() => {});

		await lsCommand.run!({
			args: {
				inbox: false,
				all: false,
				done: false,
				json: false,
				plain: false,
				_task: [],
			},
		} as never);

		const parsedContext = JSON.parse(writtenData);
		expect(parsedContext).toHaveProperty("tasks");
		expect(parsedContext).toHaveProperty("timestamp");
		expect(Array.isArray(parsedContext.tasks)).toBe(true);
		expect(parsedContext.tasks[0]).toHaveProperty("shortId", 1);
		expect(parsedContext.tasks[0]).toHaveProperty("id", "task-1");
		expect(parsedContext.tasks[0]).toHaveProperty("title");

		consoleLogSpy.mockRestore();
		getTasksMock.mockRestore();
		mkdirMock.mockRestore();
		writeFileMock.mockRestore();
	});

	it("displays error message when API request fails", async () => {
		const getTasksMock = spyOn(
			AkiflowClient.prototype,
			"getTasks",
		).mockRejectedValue(new Error("Network error"));

		const consoleErrorSpy = spyOn(console, "error").mockImplementation(
			() => {},
		);
		const processExitSpy = spyOn(process, "exit").mockImplementation(() => {
			throw new Error("Exit called");
		});

		try {
			await lsCommand.run!({
				args: {
					inbox: false,
					all: false,
					done: false,
					json: false,
					plain: false,
					_task: [],
				},
			} as never);
		} catch (error) {
			expect(error instanceof Error && error.message === "Exit called").toBe(
				true,
			);
		}

		expect(consoleErrorSpy).toHaveBeenCalledWith("Error: Network error");

		consoleErrorSpy.mockRestore();
		processExitSpy.mockRestore();
		getTasksMock.mockRestore();
	});
});

describe("getTaskDisplayTitle", () => {
	const createTask = (overrides: Partial<Task>): Task => ({
		id: "test-task",
		user_id: 1,
		recurring_id: null,
		title: null,
		description: null,
		date: null,
		datetime: null,
		datetime_tz: null,
		original_date: null,
		original_datetime: null,
		duration: null,
		recurrence: null,
		recurrence_version: null,
		status: 0,
		priority: null,
		dailyGoal: null,
		done: false,
		done_at: null,
		read_at: null,
		listId: null,
		section_id: null,
		tags_ids: [],
		sorting: 0,
		sorting_label: null,
		origin: null,
		due_date: null,
		connector_id: null,
		origin_id: null,
		origin_account_id: null,
		akiflow_account_id: null,
		doc: {},
		calendar_id: null,
		time_slot_id: null,
		links: [],
		content: {},
		trashed_at: null,
		plan_unit: null,
		plan_period: null,
		global_list_id_updated_at: null,
		global_tags_ids_updated_at: null,
		global_created_at: "2024-01-01T00:00:00Z",
		global_updated_at: "2024-01-01T00:00:00Z",
		data: {},
		deleted_at: null,
		...overrides,
	});

	it("returns title when title is available", () => {
		// given
		const task = createTask({ title: "My Task Title" });

		// when
		const result = getTaskDisplayTitle(task);

		// then
		expect(result).toBe("My Task Title");
	});

	it("returns doc.original_message with Slack prefix when title is null", () => {
		// given
		const task = createTask({
			title: null,
			connector_id: "slack",
			doc: {
				original_message:
					"<@U065YV4E7LK>\n제안서 크리에이터 ID 검증하는 API가...",
			},
		});

		// when
		const result = getTaskDisplayTitle(task);

		// then
		expect(result).toContain("[Slack]");
		expect(result).toContain("제안서 크리에이터 ID 검증하는 API가");
	});

	it("returns doc.original_message without prefix when connector_id is null", () => {
		// given
		const task = createTask({
			title: null,
			connector_id: null,
			doc: { original_message: "Some message content" },
		});

		// when
		const result = getTaskDisplayTitle(task);

		// then
		expect(result).toBe("Some message content");
		expect(result).not.toContain("[");
	});

	it("returns description when title and doc.original_message are null", () => {
		// given
		const task = createTask({
			title: null,
			doc: {},
			description: "Task description here",
		});

		// when
		const result = getTaskDisplayTitle(task);

		// then
		expect(result).toBe("Task description here");
	});

	it("returns (No title) when all fallbacks are exhausted", () => {
		// given
		const task = createTask({
			title: null,
			doc: {},
			description: null,
		});

		// when
		const result = getTaskDisplayTitle(task);

		// then
		expect(result).toBe("(No title)");
	});

	it("truncates long doc.original_message to maxLength", () => {
		// given
		const longMessage = "A".repeat(100);
		const task = createTask({
			title: null,
			connector_id: "slack",
			doc: { original_message: longMessage },
		});

		// when
		const result = getTaskDisplayTitle(task, 50);

		// then
		expect(result.length).toBe(50);
		expect(result).toContain("...");
	});

	it("replaces newlines with spaces in doc.original_message", () => {
		// given
		const task = createTask({
			title: null,
			doc: { original_message: "Line 1\nLine 2\nLine 3" },
		});

		// when
		const result = getTaskDisplayTitle(task);

		// then
		expect(result).toBe("Line 1 Line 2 Line 3");
		expect(result).not.toContain("\n");
	});

	it("handles GitHub connector prefix", () => {
		// given
		const task = createTask({
			title: null,
			connector_id: "github",
			doc: { original_message: "Issue #123: Bug fix" },
		});

		// when
		const result = getTaskDisplayTitle(task);

		// then
		expect(result).toBe("[GitHub] Issue #123: Bug fix");
	});

	it("handles unknown connector with generic prefix", () => {
		// given
		const task = createTask({
			title: null,
			connector_id: "unknown-connector",
			doc: { original_message: "Some message" },
		});

		// when
		const result = getTaskDisplayTitle(task);

		// then
		expect(result).toBe("[unknown-connector] Some message");
	});

	it("skips empty title and uses fallback", () => {
		// given
		const task = createTask({
			title: "   ",
			doc: { original_message: "Fallback message" },
		});

		// when
		const result = getTaskDisplayTitle(task);

		// then
		expect(result).toBe("Fallback message");
	});

	it("skips empty doc.original_message and uses description", () => {
		// given
		const task = createTask({
			title: null,
			doc: { original_message: "   " },
			description: "Description fallback",
		});

		// when
		const result = getTaskDisplayTitle(task);

		// then
		expect(result).toBe("Description fallback");
	});
});

describe("ls command search functionality", () => {
	const searchTasks: Task[] = [
		{
			id: "search-task-1",
			user_id: 1,
			recurring_id: null,
			title: "연말정산 서류 준비",
			description: null,
			date: todayDateString,
			datetime: null,
			datetime_tz: null,
			original_date: null,
			original_datetime: null,
			duration: null,
			recurrence: null,
			recurrence_version: null,
			status: 0,
			priority: 1,
			dailyGoal: null,
			done: false,
			done_at: null,
			read_at: null,
			listId: null,
			section_id: null,
			tags_ids: [],
			sorting: 0,
			sorting_label: null,
			origin: null,
			due_date: null,
			connector_id: null,
			origin_id: null,
			origin_account_id: null,
			akiflow_account_id: null,
			doc: {},
			calendar_id: null,
			time_slot_id: null,
			links: [],
			content: {},
			trashed_at: null,
			plan_unit: null,
			plan_period: null,
			global_list_id_updated_at: null,
			global_tags_ids_updated_at: null,
			global_created_at: "2024-01-01T00:00:00Z",
			global_updated_at: "2024-01-01T00:00:00Z",
			data: {},
			deleted_at: null,
		},
		{
			id: "search-task-2",
			user_id: 1,
			recurring_id: null,
			title: "Complete project",
			description: "연말정산 관련 프로젝트",
			date: todayDateString,
			datetime: null,
			datetime_tz: null,
			original_date: null,
			original_datetime: null,
			duration: null,
			recurrence: null,
			recurrence_version: null,
			status: 0,
			priority: 1,
			dailyGoal: null,
			done: true,
			done_at: "2024-01-01T10:00:00Z",
			read_at: null,
			listId: null,
			section_id: null,
			tags_ids: [],
			sorting: 1,
			sorting_label: null,
			origin: null,
			due_date: null,
			connector_id: null,
			origin_id: null,
			origin_account_id: null,
			akiflow_account_id: null,
			doc: {},
			calendar_id: null,
			time_slot_id: null,
			links: [],
			content: {},
			trashed_at: null,
			plan_unit: null,
			plan_period: null,
			global_list_id_updated_at: null,
			global_tags_ids_updated_at: null,
			global_created_at: "2024-01-01T00:00:00Z",
			global_updated_at: "2024-01-01T00:00:00Z",
			data: {},
			deleted_at: null,
		},
		{
			id: "search-task-3",
			user_id: 1,
			recurring_id: null,
			title: null,
			description: null,
			date: tomorrowDateString,
			datetime: null,
			datetime_tz: null,
			original_date: null,
			original_datetime: null,
			duration: null,
			recurrence: null,
			recurrence_version: null,
			status: 0,
			priority: 1,
			dailyGoal: null,
			done: false,
			done_at: null,
			read_at: null,
			listId: null,
			section_id: null,
			tags_ids: [],
			sorting: 2,
			sorting_label: null,
			origin: null,
			due_date: null,
			connector_id: "slack",
			origin_id: null,
			origin_account_id: null,
			akiflow_account_id: null,
			doc: { original_message: "연말정산 안내 메시지" },
			calendar_id: null,
			time_slot_id: null,
			links: [],
			content: {},
			trashed_at: null,
			plan_unit: null,
			plan_period: null,
			global_list_id_updated_at: null,
			global_tags_ids_updated_at: null,
			global_created_at: "2024-01-01T00:00:00Z",
			global_updated_at: "2024-01-01T00:00:00Z",
			data: {},
			deleted_at: null,
		},
		{
			id: "search-task-4",
			user_id: 1,
			recurring_id: null,
			title: "Unrelated task",
			description: null,
			date: todayDateString,
			datetime: null,
			datetime_tz: null,
			original_date: null,
			original_datetime: null,
			duration: null,
			recurrence: null,
			recurrence_version: null,
			status: 0,
			priority: 1,
			dailyGoal: null,
			done: false,
			done_at: null,
			read_at: null,
			listId: null,
			section_id: null,
			tags_ids: [],
			sorting: 3,
			sorting_label: null,
			origin: null,
			due_date: null,
			connector_id: null,
			origin_id: null,
			origin_account_id: null,
			akiflow_account_id: null,
			doc: {},
			calendar_id: null,
			time_slot_id: null,
			links: [],
			content: {},
			trashed_at: null,
			plan_unit: null,
			plan_period: null,
			global_list_id_updated_at: null,
			global_tags_ids_updated_at: null,
			global_created_at: "2024-01-01T00:00:00Z",
			global_updated_at: "2024-01-01T00:00:00Z",
			data: {},
			deleted_at: null,
		},
	];

	it("searches tasks by Korean title", async () => {
		// given
		const getTasksMock = spyOn(
			AkiflowClient.prototype,
			"getTasks",
		).mockResolvedValue({ success: true, message: null, data: searchTasks });

		const mkdirMock = spyOn(fs, "mkdir").mockResolvedValue(undefined);
		const writeFileMock = spyOn(fs, "writeFile").mockResolvedValue(undefined);
		const consoleLogSpy = spyOn(console, "log").mockImplementation(() => {});

		// when
		await lsCommand.run!({
			args: {
				inbox: false,
				all: false,
				done: false,
				search: "연말정산",
				json: false,
				plain: true,
				_task: [],
			},
		} as never);

		// then
		const consoleOutput = consoleLogSpy.mock.calls[0]?.[0] as
			| string
			| undefined;
		expect(consoleOutput).toContain("연말정산 서류 준비");
		expect(consoleOutput).toContain("Complete project");
		expect(consoleOutput).toContain("[Slack]");
		expect(consoleOutput).not.toContain("Unrelated task");

		consoleLogSpy.mockRestore();
		getTasksMock.mockRestore();
		mkdirMock.mockRestore();
		writeFileMock.mockRestore();
	});

	it("searches tasks case-insensitively", async () => {
		// given
		const getTasksMock = spyOn(
			AkiflowClient.prototype,
			"getTasks",
		).mockResolvedValue({ success: true, message: null, data: searchTasks });

		const mkdirMock = spyOn(fs, "mkdir").mockResolvedValue(undefined);
		const writeFileMock = spyOn(fs, "writeFile").mockResolvedValue(undefined);
		const consoleLogSpy = spyOn(console, "log").mockImplementation(() => {});

		// when
		await lsCommand.run!({
			args: {
				inbox: false,
				all: false,
				done: false,
				search: "PROJECT",
				json: false,
				plain: true,
				_task: [],
			},
		} as never);

		// then
		const consoleOutput = consoleLogSpy.mock.calls[0]?.[0] as
			| string
			| undefined;
		expect(consoleOutput).toContain("Complete project");

		consoleLogSpy.mockRestore();
		getTasksMock.mockRestore();
		mkdirMock.mockRestore();
		writeFileMock.mockRestore();
	});

	it("uses getTasks when search is provided", async () => {
		// given
		const getTasksMock = spyOn(
			AkiflowClient.prototype,
			"getTasks",
		).mockResolvedValue({ success: true, message: null, data: searchTasks });

		const mkdirMock = spyOn(fs, "mkdir").mockResolvedValue(undefined);
		const writeFileMock = spyOn(fs, "writeFile").mockResolvedValue(undefined);
		const consoleLogSpy = spyOn(console, "log").mockImplementation(() => {});

		// when
		await lsCommand.run!({
			args: {
				inbox: false,
				all: false,
				done: false,
				search: "test",
				json: false,
				plain: true,
				_task: [],
			},
		} as never);

		// then
		expect(getTasksMock).toHaveBeenCalled();

		consoleLogSpy.mockRestore();
		getTasksMock.mockRestore();
		mkdirMock.mockRestore();
		writeFileMock.mockRestore();
	});

	it("searches in doc.original_message content", async () => {
		// given
		const getTasksMock = spyOn(
			AkiflowClient.prototype,
			"getTasks",
		).mockResolvedValue({ success: true, message: null, data: searchTasks });

		const mkdirMock = spyOn(fs, "mkdir").mockResolvedValue(undefined);
		const writeFileMock = spyOn(fs, "writeFile").mockResolvedValue(undefined);
		const consoleLogSpy = spyOn(console, "log").mockImplementation(() => {});

		// when
		await lsCommand.run!({
			args: {
				inbox: false,
				all: false,
				done: false,
				search: "안내 메시지",
				json: false,
				plain: true,
				_task: [],
			},
		} as never);

		// then
		const consoleOutput = consoleLogSpy.mock.calls[0]?.[0] as
			| string
			| undefined;
		expect(consoleOutput).toContain("[Slack]");
		expect(consoleOutput).toContain("연말정산 안내 메시지");

		consoleLogSpy.mockRestore();
		getTasksMock.mockRestore();
		mkdirMock.mockRestore();
		writeFileMock.mockRestore();
	});

	it("returns no tasks when search term not found", async () => {
		// given
		const getTasksMock = spyOn(
			AkiflowClient.prototype,
			"getTasks",
		).mockResolvedValue({ success: true, message: null, data: searchTasks });

		const mkdirMock = spyOn(fs, "mkdir").mockResolvedValue(undefined);
		const writeFileMock = spyOn(fs, "writeFile").mockResolvedValue(undefined);
		const consoleLogSpy = spyOn(console, "log").mockImplementation(() => {});

		// when
		await lsCommand.run!({
			args: {
				inbox: false,
				all: false,
				done: false,
				search: "nonexistent",
				json: false,
				plain: true,
				_task: [],
			},
		} as never);

		// then
		const consoleOutput = consoleLogSpy.mock.calls[0]?.[0] as
			| string
			| undefined;
		expect(consoleOutput).toContain("No tasks found");

		consoleLogSpy.mockRestore();
		getTasksMock.mockRestore();
		mkdirMock.mockRestore();
		writeFileMock.mockRestore();
	});
});

describe("ls command with title fallback", () => {
	it("displays Slack message content instead of (No title)", async () => {
		// given
		const slackTask: Task = {
			id: "slack-task",
			user_id: 1,
			recurring_id: null,
			title: null,
			description: null,
			date: todayDateString,
			datetime: null,
			datetime_tz: null,
			original_date: null,
			original_datetime: null,
			duration: null,
			recurrence: null,
			recurrence_version: null,
			status: 0,
			priority: 1,
			dailyGoal: null,
			done: false,
			done_at: null,
			read_at: null,
			listId: null,
			section_id: null,
			tags_ids: [],
			sorting: 0,
			sorting_label: null,
			origin: null,
			due_date: null,
			connector_id: "slack",
			origin_id: null,
			origin_account_id: null,
			akiflow_account_id: null,
			doc: { original_message: "제안서 크리에이터 ID 검증하는 API가..." },
			calendar_id: null,
			time_slot_id: null,
			links: [],
			content: {},
			trashed_at: null,
			plan_unit: null,
			plan_period: null,
			global_list_id_updated_at: null,
			global_tags_ids_updated_at: null,
			global_created_at: "2024-01-01T00:00:00Z",
			global_updated_at: "2024-01-01T00:00:00Z",
			data: {},
			deleted_at: null,
		};

		const getTasksMock = spyOn(
			AkiflowClient.prototype,
			"getTasks",
		).mockResolvedValue({
			success: true,
			message: null,
			data: [slackTask],
		});

		const mkdirMock = spyOn(fs, "mkdir").mockResolvedValue(undefined);
		const writeFileMock = spyOn(fs, "writeFile").mockResolvedValue(undefined);
		const consoleLogSpy = spyOn(console, "log").mockImplementation(() => {});

		// when
		await lsCommand.run!({
			args: {
				inbox: false,
				all: false,
				done: false,
				json: false,
				plain: true,
				_task: [],
			},
		} as never);

		// then
		const consoleOutput = consoleLogSpy.mock.calls[0]?.[0] as
			| string
			| undefined;
		expect(consoleOutput).toContain("[Slack]");
		expect(consoleOutput).toContain("제안서 크리에이터 ID 검증하는 API가");
		expect(consoleOutput).not.toContain("(No title)");

		consoleLogSpy.mockRestore();
		getTasksMock.mockRestore();
		mkdirMock.mockRestore();
		writeFileMock.mockRestore();
	});
});
