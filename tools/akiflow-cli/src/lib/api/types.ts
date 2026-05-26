export class AuthError extends Error {
	constructor(message = "Authentication failed") {
		super(message);
		this.name = "AuthError";
	}
}

export class NetworkError extends Error {
	public readonly originalError?: Error;

	constructor(message: string, originalError?: Error) {
		super(message);
		this.name = "NetworkError";
		this.originalError = originalError;
	}
}

export interface ApiResponse<TData> {
	success: boolean;
	message: string | null;
	data: TData;
	sync_token?: string;
	has_next_page?: boolean;
}

// Real values observed in /v5/tasks (HAR capture 2026-05-21, 544 tasks):
//   1  = inbox / triage
//   2  = planned (done is the boolean — status stays 2 when completed)
//   9  = deleted tombstone (arrives with deleted_at != null)
//   10 = trashed (arrives with trashed_at != null)
//
// `0` is retained as an internal marker used by upstream code for
// virtual recurring-task instances + a buggy "Deleted" display branch
// in commands/task/index.ts. Not produced by the API; keeping it in the
// union so we can drop the buggy call sites in their respective phases
// without thrashing this type definition.
export type TaskStatus = 0 | 1 | 2 | 9 | 10;

export type TaskState = "inbox" | "planned" | "done" | "trashed" | "deleted";

export function taskStateOf(
	task: Pick<Task, "status" | "done" | "trashed_at" | "deleted_at">,
): TaskState {
	if (task.deleted_at != null) return "deleted";
	if (task.trashed_at != null) return "trashed";
	if (task.done) return "done";
	if (task.status === 1) return "inbox";
	return "planned";
}

export interface Task {
	id: string;
	user_id: number;
	recurring_id: string | null;
	title: string | null;
	description: string | null;
	date: string | null;
	datetime: string | null;
	datetime_tz: string | null;
	original_date: string | null;
	original_datetime: string | null;
	duration: number | null;
	recurrence: string | null;
	recurrence_version: number | null;
	status: TaskStatus;
	priority: number | null;
	dailyGoal: number | null;
	done: boolean;
	done_at: string | null;
	read_at: string | null;
	listId: string | null;
	section_id: string | null;
	tags_ids: string[];
	sorting: number;
	sorting_label: number | null;
	origin: string | null;
	due_date: string | null;
	connector_id: string | null;
	origin_id: string | null;
	origin_account_id: string | null;
	akiflow_account_id: string | null;
	doc: Record<string, unknown>;
	calendar_id: string | null;
	time_slot_id: string | null;
	links: string[];
	content: Record<string, unknown>;
	trashed_at: string | null;
	// Loose-date planning buckets. plan_unit is uppercase per the API
	// ("WEEK" / "MONTH"); plan_period is a packed integer:
	//   WEEK  → YYYY*100 + ISO-week  (e.g. 202621 = 2026 W21)
	//   MONTH → YYYY*100 + month     (e.g. 202605 = 2026 May)
	plan_unit: "WEEK" | "MONTH" | null;
	plan_period: number | null;
	global_list_id_updated_at: string | null;
	global_tags_ids_updated_at: string | null;
	global_created_at: string;
	global_updated_at: string;
	data: Record<string, unknown>;
	deleted_at: string | null;
}

export interface CreateTaskPayload {
	id: string;
	title: string;
	global_created_at: string;
	global_updated_at: string;
	description?: string;
	date?: string;
	datetime?: string;
	datetime_tz?: string;
	duration?: number;
	priority?: number;
	dailyGoal?: number;
	listId?: string;
	section_id?: string;
	tags_ids?: string[];
	due_date?: string;
	links?: string[];
	content?: Record<string, unknown>;
	calendar_id?: string;
	recurrence?: string;
	status?: TaskStatus;
}

export interface UpdateTaskPayload {
	id: string;
	global_updated_at: string;
	title?: string;
	description?: string;
	date?: string;
	datetime?: string;
	datetime_tz?: string;
	duration?: number;
	priority?: number;
	dailyGoal?: number;
	listId?: string;
	section_id?: string;
	tags_ids?: string[];
	due_date?: string;
	links?: string[];
	content?: Record<string, unknown>;
	done?: boolean;
	done_at?: string | null;
	status?: TaskStatus;
	deleted_at?: string | null;
}

export interface Label {
	id: string;
	user_id: number;
	parent_id: string | null;
	title: string;
	icon: string | null;
	color: string | null;
	sorting: number;
	type: string | null;
	global_created_at: string;
	global_updated_at: string;
	data: Record<string, unknown>;
	deleted_at: string | null;
}

export interface Tag {
	id: string;
	user_id: number;
	title: string;
	color: string | null;
	sorting: number;
	global_created_at: string;
	global_updated_at: string;
	data: Record<string, unknown>;
	deleted_at: string | null;
}

export type TimeSlotStatus = "confirmed" | "tentative";

export interface TimeSlot {
	id: string;
	user_id: number;
	recurring_id: string | null;
	calendar_id: string;
	label_id: string | null;
	section_id: string | null;
	status: TimeSlotStatus;
	title: string;
	description: string | null;
	original_start_time: string | null;
	start_time: string;
	end_time: string;
	start_datetime_tz: string;
	recurrence: string | null;
	color: string | null;
	content: Record<string, unknown>;
	global_label_id_updated_at: string | null;
	global_created_at: string;
	global_updated_at: string;
	data: Record<string, unknown>;
	deleted_at: string | null;
}

export interface AkiflowCredentials {
	token: string;
	clientId: string;
	refreshToken?: string;
}

export interface TokenRefreshResponse {
	token_type: string;
	expires_in: number;
	access_token: string;
	refresh_token: string;
}

// ============================================================
// Calendar primitives (HAR capture 2026-05-21)
// ============================================================

export type EventStatus = "confirmed" | "tentative" | "cancelled";

export interface Event {
	id: string;
	user_id: number;

	// Recurrence
	recurring_id: string | null;
	recurrence_exception: boolean;
	recurrence_exception_delete: string | null;
	recurrence_sync_retry: unknown | null;
	recurrence: string[] | null;
	origin_recurring_id: string | null;

	// Timing — start_date/end_date set for all-day; start_time/end_time for timed
	start_time: string | null;
	end_time: string | null;
	start_date: string | null;
	end_date: string | null;
	start_datetime_tz: string;
	end_datetime_tz: string | null;
	original_start_time: string | null;
	original_start_date: string | null;

	// Display
	title: string | null;
	description: string | null;
	status: EventStatus;
	declined: boolean;
	read_only: boolean;
	hidden: boolean;
	color: string | null;
	calendar_color: string | null;

	// People
	attendees: unknown[] | null;
	organizer_id: string | null;
	creator_id: string | null;
	created_by: string | null;

	// Meeting
	meeting_url: string | null;
	meeting_solution: string | null;
	meeting_icon: string | null;
	meeting_status: string | null;

	// Linkage
	calendar_id: string;
	task_id: string | null;
	time_slot_id: string | null;
	url: string | null;

	// External source
	origin_id: string | null;
	origin_account_id: string | null;
	origin_calendar_id: string | null;
	origin_updated_at: string | null;
	akiflow_account_id: string | null;
	connector_id: string | null;

	// Email reminder internals (rarely useful to CLI consumers)
	availability_config_id: string | null;
	email_confirmation_type: string | null;
	email_confirmation_status: string | null;
	email_reminder_type: string | null;
	email_reminder_status: string | null;
	email_remind_before: number | null;
	email_reminded_at: string | null;

	// Akiflow internals
	content: Record<string, unknown>;
	data: Record<string, unknown>;
	fingerprints: Record<string, string>;
	etag: string | null;

	// Timestamps
	global_created_at: string;
	global_updated_at: string;
	deleted_at: string | null;
}

export interface Calendar {
	id: string;
	user_id: number;
	akiflow_account_id: string;
	akiflow_primary: boolean;
	primary: boolean;
	connector_id: string;
	origin_id: string;
	origin_account_id: string;
	title: string;
	description: string | null;
	timezone: string;
	color: string | null;
	icon: string | null;
	read_only: boolean;
	hidden_at: string | null;
	url: string | null;
	sync_status: string | null;
	last_synced_at: string | null;
	clear_job_id: string | null;
	settings: Record<string, unknown>;
	content: Record<string, unknown>;
	data: Record<string, unknown>;
	fingerprints: Record<string, string>;
	etag: string | null;
	global_created_at: string;
	global_updated_at: string;
	deleted_at: string | null;
}

export interface Account {
	id: string;
	user_id: number;
	account_id: string;
	origin_account_id: string;
	connector_id: string;
	full_name: string | null;
	short_name: string | null;
	identifier: string; // email
	picture: string | null;
	status: string;
	sync_status: string | null;
	autologin_token: string | null;
	details: Record<string, unknown>;
	data: Record<string, unknown>;
	global_created_at: string;
	global_updated_at: string;
	deleted_at: string | null;
}

export interface Contact {
	id: string;
	user_id: number;
	akiflow_account_id: string;
	connector_id: string;
	origin_id: string;
	origin_account_id: string;
	name: string | null;
	identifier: string; // email
	picture: string | null;
	url: string | null;
	local_url: string | null;
	search_text: string;
	sorting: number;
	content: Record<string, unknown>;
	etag: string | null;
	origin_updated_at: string | null;
	global_created_at: string;
	global_updated_at: string;
	deleted_at: string | null;
}
