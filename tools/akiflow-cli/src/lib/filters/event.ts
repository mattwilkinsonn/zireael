import type { Event, Task, TimeSlot } from "../api/types";

export interface EventFilter {
	from?: Date;
	to?: Date;
	/** Calendar id (resolved from name → id by caller). */
	calendar?: string;
	/** Akiflow account id. */
	account?: string;
	/** "google" | "microsoft" | "icloud" | ... */
	connector?: string;
	includeDeclined?: boolean;
	allDayOnly?: boolean;
	noAllDay?: boolean;
}

export function filterEvents(events: Event[], f: EventFilter): Event[] {
	return events.filter((e) => {
		if (e.deleted_at != null) return false;
		if (!f.includeDeclined && e.declined) return false;
		const isAllDay = e.start_date != null;
		if (f.allDayOnly && !isAllDay) return false;
		if (f.noAllDay && isAllDay) return false;
		if (f.from && f.to) {
			const ref = e.start_time ?? e.start_date;
			if (!ref) return false;
			const startMs = new Date(ref).getTime();
			if (startMs < f.from.getTime() || startMs > f.to.getTime()) return false;
		}
		if (f.calendar && e.calendar_id !== f.calendar) return false;
		if (f.account && e.akiflow_account_id !== f.account) return false;
		if (f.connector && e.connector_id !== f.connector) return false;
		return true;
	});
}

export type TimelineEntry =
	| { type: "event"; record: Event; start: Date; end: Date | null }
	| { type: "time_slot"; record: TimeSlot; start: Date; end: Date | null }
	| { type: "task"; record: Task; start: Date; end: Date | null };

/**
 * Merge events, time slots, and tasks-with-datetime into a single
 * timeline sorted by start time. Dedup: when an Event has task_id or
 * time_slot_id set, the linked Task/Slot is skipped (Event is canonical).
 */
export function mergeTimeline(
	events: Event[],
	slots: TimeSlot[],
	tasks: Task[],
): TimelineEntry[] {
	const linkedTaskIds = new Set(
		events.map((e) => e.task_id).filter((id): id is string => !!id),
	);
	const linkedSlotIds = new Set(
		events.map((e) => e.time_slot_id).filter((id): id is string => !!id),
	);

	const result: TimelineEntry[] = [];

	for (const e of events) {
		const ref = e.start_time ?? e.start_date;
		if (!ref) continue;
		const start = new Date(ref);
		const endRef = e.end_time ?? e.end_date;
		const end = endRef ? new Date(endRef) : null;
		result.push({ type: "event", record: e, start, end });
	}

	for (const s of slots) {
		if (linkedSlotIds.has(s.id)) continue;
		result.push({
			type: "time_slot",
			record: s,
			start: new Date(s.start_time),
			end: new Date(s.end_time),
		});
	}

	for (const t of tasks) {
		if (!t.datetime) continue;
		if (linkedTaskIds.has(t.id)) continue;
		const start = new Date(t.datetime);
		const end = t.duration
			? new Date(start.getTime() + t.duration * 60_000)
			: null;
		result.push({ type: "task", record: t, start, end });
	}

	result.sort((a, b) => a.start.getTime() - b.start.getTime());
	return result;
}
