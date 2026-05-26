import type { Task, TaskState } from "../api/types";
import { taskStateOf } from "../api/types";

export type StatusName = TaskState | "active" | "all";

export interface TaskFilter {
	/**
	 * Status names: inbox | planned | done | trashed | active | all.
	 * Default: ["inbox","planned"] (= active). Tombstones (status=9) are
	 * never returned regardless.
	 */
	status?: StatusName[];
	from?: Date;
	to?: Date;
	overdue?: boolean;
	project?: string;
	tag?: string;
	priority?: number;
	/** "gmail" | "linear" | "akiflow" | "none". */
	connector?: string;
	bucket?: "week" | "month";
	recurring?: boolean;
	unplanned?: boolean;
	planned?: boolean;
}

const DEFAULT_STATUSES: StatusName[] = ["inbox", "planned"];

export function filterTasks(tasks: Task[], f: TaskFilter): Task[] {
	const statusNames: StatusName[] = f.status ?? DEFAULT_STATUSES;
	const expanded = expandStatusNames(statusNames);
	return tasks.filter((t) => {
		if (!expanded.has(taskStateOf(t))) return false;
		if (f.from && f.to && !inDateRange(t, f.from, f.to)) return false;
		if (f.overdue && !isOverdue(t)) return false;
		if (f.project && t.listId !== f.project) return false;
		if (f.tag && !t.tags_ids.includes(f.tag)) return false;
		if (f.priority != null && t.priority !== f.priority) return false;
		if (f.connector && !matchConnector(t, f.connector)) return false;
		if (f.bucket && !matchBucket(t, f.bucket)) return false;
		if (f.recurring && !t.recurring_id) return false;
		if (f.unplanned && !isUnplanned(t)) return false;
		if (f.planned && isUnplanned(t)) return false;
		return true;
	});
}

function expandStatusNames(names: StatusName[]): Set<TaskState> {
	const out = new Set<TaskState>();
	for (const n of names) {
		if (n === "active") {
			out.add("inbox");
			out.add("planned");
		} else if (n === "all") {
			out.add("inbox");
			out.add("planned");
			out.add("done");
			out.add("trashed");
		} else if (n !== "deleted") {
			out.add(n);
		}
	}
	return out;
}

function inDateRange(t: Task, from: Date, to: Date): boolean {
	const fromMs = startOfDay(from).getTime();
	const toMs = endOfDay(to).getTime();
	if (t.date) {
		const d = new Date(t.date).getTime();
		if (d >= fromMs && d <= toMs) return true;
	}
	if (t.datetime) {
		const d = new Date(t.datetime).getTime();
		if (d >= fromMs && d <= toMs) return true;
	}
	if (t.plan_unit === "WEEK" && t.plan_period != null) {
		return weekBucketIntersects(t.plan_period, fromMs, toMs);
	}
	if (t.plan_unit === "MONTH" && t.plan_period != null) {
		return monthBucketIntersects(t.plan_period, fromMs, toMs);
	}
	return false;
}

function weekBucketIntersects(
	period: number,
	fromMs: number,
	toMs: number,
): boolean {
	const year = Math.floor(period / 100);
	const week = period % 100;
	const { weekStart, weekEnd } = isoWeekRange(year, week);
	return weekStart.getTime() <= toMs && weekEnd.getTime() >= fromMs;
}

function monthBucketIntersects(
	period: number,
	fromMs: number,
	toMs: number,
): boolean {
	const year = Math.floor(period / 100);
	const month = period % 100;
	const monthStart = new Date(year, month - 1, 1);
	const monthEnd = new Date(year, month, 0, 23, 59, 59, 999);
	return monthStart.getTime() <= toMs && monthEnd.getTime() >= fromMs;
}

function isoWeekRange(
	year: number,
	week: number,
): { weekStart: Date; weekEnd: Date } {
	// ISO week 1 contains the first Thursday of the year
	const jan4 = new Date(year, 0, 4);
	const jan4Dow = (jan4.getDay() + 6) % 7;
	const week1Monday = new Date(jan4);
	week1Monday.setDate(jan4.getDate() - jan4Dow);
	const weekStart = new Date(week1Monday);
	weekStart.setDate(week1Monday.getDate() + (week - 1) * 7);
	const weekEnd = new Date(weekStart);
	weekEnd.setDate(weekStart.getDate() + 6);
	weekEnd.setHours(23, 59, 59, 999);
	return { weekStart, weekEnd };
}

function startOfDay(d: Date): Date {
	return new Date(d.getFullYear(), d.getMonth(), d.getDate(), 0, 0, 0, 0);
}
function endOfDay(d: Date): Date {
	return new Date(d.getFullYear(), d.getMonth(), d.getDate(), 23, 59, 59, 999);
}

function isOverdue(t: Task): boolean {
	if (t.done) return false;
	const ref = t.datetime ?? t.date;
	if (!ref) return false;
	return new Date(ref).getTime() < startOfDay(new Date()).getTime();
}

function matchConnector(t: Task, connector: string): boolean {
	if (connector === "none") return t.connector_id == null;
	return t.connector_id === connector;
}

function matchBucket(t: Task, bucket: "week" | "month"): boolean {
	if (bucket === "week") return t.plan_unit === "WEEK";
	return t.plan_unit === "MONTH";
}

function isUnplanned(t: Task): boolean {
	return t.date == null && t.datetime == null && t.plan_unit == null;
}
