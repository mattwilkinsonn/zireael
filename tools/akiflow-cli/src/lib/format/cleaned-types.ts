import type {
	Account,
	Calendar,
	Event,
	Label,
	Task,
	TimeSlot,
} from "../api/types";
import { taskStateOf } from "../api/types";

// ============================================================
// Cleaned JSON shapes — stable contract for --json output
// ============================================================

export interface CleanedSource {
	connector: string;
	id: string;
	url: string | null;
	account: string | null;
	/** gmail-specific */
	thread_id?: string;
	subject?: string;
	from?: string;
}

export interface CleanedTaskView {
	id: string;
	status: "inbox" | "planned" | "done" | "trashed";
	title: string | null;
	description: string | null;
	date: string | null;
	datetime: string | null;
	duration_min: number | null;
	due_date: string | null;
	plan_bucket: { unit: "week" | "month"; period: string } | null;
	overdue: boolean;
	priority: number | null;
	project: string | null;
	project_id: string | null;
	tags: string[];
	source: CleanedSource | null;
	recurring: { id: string; rule: string } | null;
	linked_event_id: string | null;
	linked_slot_id: string | null;
	created_at: string;
	updated_at: string;
	done_at: string | null;
	read_at: string | null;
}

export interface CleanedCalEntry {
	id: string;
	type: "event" | "time_slot" | "task";
	title: string | null;
	description: string | null;
	start: string;
	end: string | null;
	all_day: boolean;
	timezone: string | null;
	calendar: string | null;
	calendar_id: string | null;
	account: string | null;
	status: "confirmed" | "tentative" | "cancelled" | null;
	declined: boolean | null;
	read_only: boolean | null;
	meeting: { url: string; solution: string | null } | null;
	attendees: Array<{
		email: string;
		name: string;
		response: string | null;
	}> | null;
	recurring: { id: string; rule: string } | null;
	linked_task_id: string | null;
	linked_slot_id: string | null;
	created_at: string;
	updated_at: string;
}

export interface ResolveContext {
	labelsById: Map<string, Label>;
	accountsById: Map<string, Account>;
	calendarsById: Map<string, Calendar>;
}

export function emptyContext(): ResolveContext {
	return {
		labelsById: new Map(),
		accountsById: new Map(),
		calendarsById: new Map(),
	};
}

// ============================================================
// Task → CleanedTaskView
// ============================================================

export function toCleanedTaskView(
	t: Task,
	ctx: ResolveContext,
): CleanedTaskView {
	const state = taskStateOf(t);
	return {
		id: t.id,
		// CleanedTaskView excludes "deleted" — tombstones never appear in cleaned output
		status: state === "deleted" ? "trashed" : state,
		title: t.title,
		description: t.description,
		date: t.date,
		datetime: t.datetime,
		duration_min: t.duration,
		due_date: t.due_date,
		plan_bucket: buildPlanBucket(t),
		overdue: isOverdueTask(t),
		priority: t.priority,
		project: t.listId ? (ctx.labelsById.get(t.listId)?.title ?? null) : null,
		project_id: t.listId,
		tags: t.tags_ids,
		source: extractSource(t, ctx),
		recurring:
			t.recurring_id && t.recurrence?.[0]
				? { id: t.recurring_id, rule: t.recurrence[0] }
				: null,
		linked_event_id: t.calendar_id,
		linked_slot_id: t.time_slot_id,
		created_at: t.global_created_at,
		updated_at: t.global_updated_at,
		done_at: t.done_at,
		read_at: t.read_at,
	};
}

function buildPlanBucket(
	t: Task,
): { unit: "week" | "month"; period: string } | null {
	if (!t.plan_unit || t.plan_period == null) return null;
	const year = Math.floor(t.plan_period / 100);
	const tail = t.plan_period % 100;
	const padded = String(tail).padStart(2, "0");
	if (t.plan_unit === "WEEK") {
		return { unit: "week", period: `${year}-W${padded}` };
	}
	if (t.plan_unit === "MONTH") {
		return { unit: "month", period: `${year}-${padded}` };
	}
	return null;
}

function extractSource(t: Task, ctx: ResolveContext): CleanedSource | null {
	if (!t.connector_id) return null;
	const doc = t.doc as
		| {
				url?: string;
				thread_id?: string;
				subject?: string;
				from?: string;
				account_identifier?: string;
		  }
		| undefined;
	const account = t.akiflow_account_id
		? (ctx.accountsById.get(t.akiflow_account_id)?.identifier ?? null)
		: null;
	const source: CleanedSource = {
		connector: t.connector_id,
		id: t.origin_id ?? "",
		url: doc?.url ?? null,
		account: account ?? doc?.account_identifier ?? null,
	};
	if (doc?.thread_id) source.thread_id = doc.thread_id;
	if (doc?.subject) source.subject = doc.subject;
	if (doc?.from) source.from = doc.from;
	return source;
}

function isOverdueTask(t: Task): boolean {
	if (t.done) return false;
	const ref = t.datetime ?? t.date;
	if (!ref) return false;
	const today = new Date();
	today.setHours(0, 0, 0, 0);
	return new Date(ref).getTime() < today.getTime();
}

// ============================================================
// Timeline entry → CleanedCalEntry
// ============================================================

export function toCleanedCalView(
	entry: {
		type: "event" | "time_slot" | "task";
		record: Event | TimeSlot | Task;
		start: Date;
		end: Date | null;
	},
	ctx: ResolveContext,
): CleanedCalEntry {
	const base = {
		start: entry.start.toISOString(),
		end: entry.end?.toISOString() ?? null,
	};

	if (entry.type === "event") {
		const e = entry.record as Event;
		return {
			...base,
			id: e.id,
			type: "event",
			title: e.title,
			description: e.description,
			all_day: e.start_date != null,
			timezone: e.start_datetime_tz,
			calendar: ctx.calendarsById.get(e.calendar_id)?.title ?? null,
			calendar_id: e.calendar_id,
			account: e.akiflow_account_id
				? (ctx.accountsById.get(e.akiflow_account_id)?.identifier ?? null)
				: null,
			status: e.status,
			declined: e.declined,
			read_only: e.read_only,
			meeting: e.meeting_url
				? { url: e.meeting_url, solution: e.meeting_solution }
				: null,
			attendees: extractAttendees(e.attendees),
			recurring:
				e.recurring_id && e.recurrence?.[0]
					? { id: e.recurring_id, rule: e.recurrence[0] }
					: null,
			linked_task_id: e.task_id,
			linked_slot_id: e.time_slot_id,
			created_at: e.global_created_at,
			updated_at: e.global_updated_at,
		};
	}

	if (entry.type === "time_slot") {
		const s = entry.record as TimeSlot;
		return {
			...base,
			id: s.id,
			type: "time_slot",
			title: s.title,
			description: s.description,
			all_day: false,
			timezone: s.start_datetime_tz,
			calendar: ctx.calendarsById.get(s.calendar_id)?.title ?? null,
			calendar_id: s.calendar_id,
			account: null,
			status: null,
			declined: null,
			read_only: null,
			meeting: null,
			attendees: null,
			recurring:
				s.recurring_id && s.recurrence
					? { id: s.recurring_id, rule: s.recurrence }
					: null,
			linked_task_id: null,
			linked_slot_id: null,
			created_at: s.global_created_at,
			updated_at: s.global_updated_at,
		};
	}

	// task
	const t = entry.record as Task;
	return {
		...base,
		id: t.id,
		type: "task",
		title: t.title,
		description: t.description,
		all_day: false,
		timezone: t.datetime_tz,
		calendar: null,
		calendar_id: null,
		account: null,
		status: null,
		declined: null,
		read_only: null,
		meeting: null,
		attendees: null,
		recurring: null,
		linked_task_id: null,
		linked_slot_id: null,
		created_at: t.global_created_at,
		updated_at: t.global_updated_at,
	};
}

function extractAttendees(raw: unknown[] | null): CleanedCalEntry["attendees"] {
	if (!raw) return null;
	return raw.map((a) => {
		const att = a as { email?: string; name?: string; response?: string };
		return {
			email: att.email ?? "",
			name: att.name ?? "",
			response: att.response ?? null,
		};
	});
}
