# Akiflow API Specification

**Base URL**: `https://api.akiflow.com`  
**API Version**: v5 (Tasks, Labels, Tags, Time Slots, Contacts) / v3 (Calendars, Events)  
**Reverse Engineered**: 2026-02-01

---

## Table of Contents
1. [Authentication](#authentication)
2. [Required Headers](#required-headers)
3. [Tasks API (Full CRUD)](#tasks-api)
4. [Labels API (Projects)](#labels-api)
5. [Tags API](#tags-api)
6. [Time Slots API](#time-slots-api)
7. [Calendars API](#calendars-api)
8. [Events API](#events-api)
9. [User Settings API](#user-settings-api)
10. [Sync Mechanism](#sync-mechanism)
11. [Token Extraction from Arc Browser](#token-extraction)

---

## Authentication

### Token Type
- **JWT Bearer Token** (RS256 signed)
- Token expiry: ~30 minutes from issuance

### How to Obtain Token
Akiflow uses **session-based authentication** via web cookies. The JWT token is obtained after login through the web app and stored in **IndexedDB** (`akiflow_system.internal.akiUserToken`).

For programmatic access, extract the token from browser session using CDP (Chrome DevTools Protocol) or by reading the `remember_web_*` cookie from Arc/Chrome.

---

## Required Headers

| Header | Required | Description |
|--------|----------|-------------|
| `Authorization` | Yes | `Bearer <JWT_TOKEN>` |
| `Akiflow-Client-Id` | Yes | Client UUID (stored in IndexedDB `akiflow_system.general.clientId`) |
| `Akiflow-Version` | Yes | App version (e.g., `2.66.3`) |
| `Akiflow-Platform` | Yes | Platform identifier (`mac`, `win`, `web`, `ios`, `android`) |
| `Accept` | Yes | `application/json` |
| `Content-Type` | Required for PATCH | `application/json` |

### Example Headers
```bash
-H "Authorization: Bearer eyJ0eXAiOiJKV1QiLCJhbGciOiJSUzI1NiJ9..." \
-H "Akiflow-Client-Id: d5dbdbd6-c323-40fc-ae06-73e84aa8b83d" \
-H "Akiflow-Version: 2.66.3" \
-H "Akiflow-Platform: mac" \
-H "Accept: application/json"
```

---

## Tasks API

**Endpoint**: `/v5/tasks`  
**Supported Methods**: `GET`, `PATCH` (upsert pattern - no POST/PUT/DELETE)

### Read Tasks

```bash
GET /v5/tasks?limit=2500
GET /v5/tasks?limit=2500&sync_token=<base64_token>
```

**Query Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `limit` | int | Max number of tasks to return (max: 2500) |
| `sync_token` | string | Base64 encoded token for incremental sync |

**Response:**
```json
{
  "success": true,
  "message": null,
  "data": [...],
  "sync_token": "eyJjaGFuZ2VfaWQ...",
  "has_next_page": false
}
```

### Create/Update Tasks (Upsert)

```bash
PATCH /v5/tasks
Content-Type: application/json

[
  {
    "id": "<uuid>",
    "title": "Task title",
    "global_created_at": "2026-02-01T05:39:12.000Z",
    "global_updated_at": "2026-02-01T05:39:12.000Z"
  }
]
```

**Request Body**: Array of task objects

**Required Fields for CREATE:**

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID | Client-generated UUID |
| `title` | string | Task title |
| `global_created_at` | ISO8601 | Creation timestamp |
| `global_updated_at` | ISO8601 | Update timestamp |

**Optional Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `description` | string | Task description |
| `date` | date | Scheduled date (YYYY-MM-DD) |
| `datetime` | ISO8601 | Scheduled datetime |
| `datetime_tz` | string | Timezone (e.g., "Asia/Seoul") |
| `duration` | int | Duration in seconds |
| `priority` | int | Priority level (1 = high) |
| `dailyGoal` | int | Daily goal flag (0/1) |
| `listId` | UUID | Project/Label ID |
| `section_id` | UUID | Section within project |
| `tags_ids` | UUID[] | Array of tag IDs |
| `due_date` | date | Due date |
| `links` | string[] | Array of URLs |
| `content` | object | Additional metadata |
| `calendar_id` | UUID | Linked calendar |
| `recurrence` | string | Recurrence rule (RRULE) |

### Complete Task

```bash
PATCH /v5/tasks
Content-Type: application/json

[
  {
    "id": "<uuid>",
    "done": true,
    "done_at": "2026-02-01T05:40:56.000Z",
    "status": 2,
    "global_updated_at": "2026-02-01T05:40:56.000Z"
  }
]
```

**Status Values:**
- `null` or `0`: Pending
- `1`: In Progress (assumed)
- `2`: Completed

### Uncomplete Task

```bash
PATCH /v5/tasks
Content-Type: application/json

[
  {
    "id": "<uuid>",
    "done": false,
    "done_at": null,
    "status": 0,
    "global_updated_at": "2026-02-01T05:41:00.000Z"
  }
]
```

### Delete Task (Soft Delete)

```bash
PATCH /v5/tasks
Content-Type: application/json

[
  {
    "id": "<uuid>",
    "deleted_at": "2026-02-01T05:41:52.000Z",
    "global_updated_at": "2026-02-01T05:41:52.000Z"
  }
]
```

### Task Object Schema

```typescript
interface Task {
  id: string;                          // UUID
  user_id: number;                     // User ID
  recurring_id: string | null;         // Parent recurring task ID
  title: string;                       // Task title
  description: string | null;          // Description/notes
  date: string | null;                 // Scheduled date (YYYY-MM-DD)
  datetime: string | null;             // Scheduled datetime (ISO8601)
  datetime_tz: string | null;          // Timezone
  original_date: string | null;        // Original date (for rescheduled)
  original_datetime: string | null;    // Original datetime
  duration: number | null;             // Duration in seconds
  recurrence: string | null;           // RRULE string
  recurrence_version: number | null;   // Recurrence version
  status: number | null;               // Status (0=pending, 2=done)
  priority: number | null;             // Priority (1=high)
  dailyGoal: number | null;            // Daily goal flag
  done: boolean;                       // Completion status
  done_at: string | null;              // Completion timestamp
  read_at: string | null;              // Read timestamp
  listId: string | null;               // Project/Label UUID
  section_id: string | null;           // Section UUID
  tags_ids: string[];                  // Tag UUIDs
  sorting: number;                     // Sort order (timestamp-based)
  sorting_label: number | null;        // Sort within label
  origin: string | null;               // Source type
  due_date: string | null;             // Due date
  connector_id: string | null;         // Integration connector (slack, notion, etc.)
  origin_id: string | null;            // External ID
  origin_account_id: string | null;    // External account ID
  akiflow_account_id: string | null;   // Linked Akiflow account
  doc: object;                         // Integration-specific data
  calendar_id: string | null;          // Calendar UUID
  time_slot_id: string | null;         // Time slot UUID
  links: string[];                     // Attached URLs
  content: object;                     // AI predictions, event settings
  trashed_at: string | null;           // Trash timestamp
  plan_unit: string | null;            // Planning unit
  plan_period: string | null;          // Planning period
  global_list_id_updated_at: string | null;
  global_tags_ids_updated_at: string | null;
  global_created_at: string;           // Creation timestamp
  global_updated_at: string;           // Update timestamp
  data: object;                        // Additional data
  deleted_at: string | null;           // Soft delete timestamp
}
```

---

## Labels API

**Endpoint**: `/v5/labels`  
**Supported Methods**: `GET`, `PATCH`

Labels represent **Projects** in the Akiflow UI.

### Read Labels

```bash
GET /v5/labels?limit=2500
GET /v5/labels?limit=2500&sync_token=<token>
```

### Label Object Schema

```typescript
interface Label {
  id: string;                    // UUID
  user_id: number;               // User ID
  parent_id: string | null;      // Parent label (for folders)
  title: string;                 // Project name
  icon: string | null;           // Icon identifier
  color: string | null;          // Color palette (e.g., "palette-blueberry")
  sorting: number;               // Sort order
  type: string | null;           // Label type
  global_created_at: string;     // Creation timestamp
  global_updated_at: string;     // Update timestamp
  data: object;                  // Additional data
  deleted_at: string | null;     // Soft delete timestamp
}
```

**Available Colors:**
- `palette-blueberry`
- `palette-cobalt`
- `palette-violet`
- `palette-grape`
- `palette-tangerine`
- `palette-mint`
- etc.

---

## Tags API

**Endpoint**: `/v5/tags`  
**Supported Methods**: `GET`, `PATCH`

### Read Tags

```bash
GET /v5/tags?limit=2500
GET /v5/tags?limit=2500&sync_token=<token>
```

---

## Time Slots API

**Endpoint**: `/v5/time_slots`  
**Supported Methods**: `GET`, `PATCH`

Time slots represent **calendar blocks** in the planner.

### Read Time Slots

```bash
GET /v5/time_slots?limit=2500
GET /v5/time_slots?limit=2500&sync_token=<token>
```

### Time Slot Object Schema

```typescript
interface TimeSlot {
  id: string;                      // UUID
  user_id: number;                 // User ID
  recurring_id: string | null;     // Parent recurring slot
  calendar_id: string;             // Calendar UUID
  label_id: string | null;         // Linked project
  section_id: string | null;       // Section
  status: string;                  // "confirmed" | "tentative"
  title: string;                   // Slot title
  description: string | null;      // Description
  original_start_time: string | null;
  start_time: string;              // Start datetime (ISO8601)
  end_time: string;                // End datetime (ISO8601)
  start_datetime_tz: string;       // Timezone
  recurrence: string | null;       // RRULE
  color: string | null;            // Color override
  content: object;                 // Additional data
  global_label_id_updated_at: string | null;
  global_created_at: string;       // Creation timestamp
  global_updated_at: string;       // Update timestamp
  data: object;                    // Additional data
  deleted_at: string | null;       // Soft delete timestamp
}
```

---

## Calendars API

**Endpoint**: `/v3/calendars`  
**Supported Methods**: `GET`

### Read Calendars

```bash
GET /v3/calendars?per_page=2500&with_deleted=false
GET /v3/calendars?per_page=2500&with_deleted=true&updatedAfter=<ISO8601>
```

---

## Events API

**Endpoint**: `/v3/events`  
**Supported Methods**: `GET`

### Read Events

```bash
GET /v3/events?per_page=2500&with_deleted=false
GET /v3/events?per_page=2500&with_deleted=true&updatedAfter=<ISO8601>
GET /v3/events?cursor=<base64>&with_deleted=false&per_page=2500
```

### Event Modifiers

```bash
GET /v3/events/modifiers?per_page=2500&with_deleted=false
```

---

## User Settings API

**Endpoint**: `/v5/user/settings`  
**Supported Methods**: `GET`, `PATCH`

### Read Settings

```bash
GET /v5/user/settings
```

### Update Settings

```bash
PATCH /v5/user/settings
Content-Type: application/json

{
  "client_id": "<uuid>",
  "settings": {
    "general": [
      {"key": "timezoneOffset", "value": -540, "updatedAt": 1769924267129},
      {"key": "timezoneName", "value": "Asia/Seoul", "updatedAt": 1769924267129}
    ],
    "shareAvailability": [
      {"key": "baseUrlPath", "value": "", "updatedAt": 1769924267128}
    ]
  }
}
```

---

## Sync Mechanism

Akiflow uses an **optimistic offline-first architecture** with incremental sync:

1. **Initial Sync**: `GET /v5/tasks?limit=2500` (no sync_token)
2. **Incremental Sync**: `GET /v5/tasks?limit=2500&sync_token=<token>`
3. **Sync Token**: Base64 encoded JSON containing `change_id` and `ts` (timestamp)

### Sync Token Format

```json
{
  "change_id": 691444480,
  "ts": "2026-02-01T05:34:05.589885Z"
}
```

The server returns a new `sync_token` with each response. Use it for the next request to get only changed items.

---

## Token Extraction

### From Arc Browser (Python)

```python
import browser_cookie3

# Get Akiflow cookies from Arc
cj = browser_cookie3.arc(domain_name='akiflow.com')

# The remember_web_* cookie authenticates the session
for cookie in cj:
    if 'remember_web_' in cookie.name:
        print(f"Session Cookie: {cookie.value}")
```

### From Playwright/CDP

```javascript
// After login, capture API requests via CDP
const client = await page.context().newCDPSession(page);
await client.send('Network.enable');

client.on('Network.requestWillBeSent', (params) => {
  if (params.request.url.includes('api.akiflow.com')) {
    console.log('Authorization:', params.request.headers.Authorization);
    console.log('Client-Id:', params.request.headers['Akiflow-Client-Id']);
  }
});
```

### From IndexedDB (Browser Console)

```javascript
// Open akiflow_system database
const request = indexedDB.open('akiflow_system');
request.onsuccess = (e) => {
  const db = e.target.result;
  
  // Get client ID
  const tx1 = db.transaction(['general'], 'readonly');
  const store1 = tx1.objectStore('general');
  store1.getAll().onsuccess = (e) => console.log('General:', e.target.result);
  
  // Get internal data (includes aki tokens)
  const tx2 = db.transaction(['internal'], 'readonly');
  const store2 = tx2.objectStore('internal');
  store2.getAll().onsuccess = (e) => console.log('Internal:', e.target.result);
};
```

---

## curl Examples

### Create Task

```bash
curl -X PATCH "https://api.akiflow.com/v5/tasks" \
  -H "Authorization: Bearer <token>" \
  -H "Akiflow-Client-Id: <uuid>" \
  -H "Akiflow-Version: 2.66.3" \
  -H "Akiflow-Platform: mac" \
  -H "Content-Type: application/json" \
  -H "Accept: application/json" \
  -d '[{
    "id": "'"$(uuidgen | tr '[:upper:]' '[:lower:]')"'",
    "title": "My new task",
    "date": "2026-02-01",
    "global_created_at": "'"$(date -u +%Y-%m-%dT%H:%M:%S.000Z)"'",
    "global_updated_at": "'"$(date -u +%Y-%m-%dT%H:%M:%S.000Z)"'"
  }]'
```

### Complete Task

```bash
curl -X PATCH "https://api.akiflow.com/v5/tasks" \
  -H "Authorization: Bearer <token>" \
  -H "Akiflow-Client-Id: <uuid>" \
  -H "Akiflow-Version: 2.66.3" \
  -H "Akiflow-Platform: mac" \
  -H "Content-Type: application/json" \
  -d '[{
    "id": "<task-uuid>",
    "done": true,
    "done_at": "'"$(date -u +%Y-%m-%dT%H:%M:%S.000Z)"'",
    "status": 2,
    "global_updated_at": "'"$(date -u +%Y-%m-%dT%H:%M:%S.000Z)"'"
  }]'
```

### Delete Task

```bash
curl -X PATCH "https://api.akiflow.com/v5/tasks" \
  -H "Authorization: Bearer <token>" \
  -H "Akiflow-Client-Id: <uuid>" \
  -H "Akiflow-Version: 2.66.3" \
  -H "Akiflow-Platform: mac" \
  -H "Content-Type: application/json" \
  -d '[{
    "id": "<task-uuid>",
    "deleted_at": "'"$(date -u +%Y-%m-%dT%H:%M:%S.000Z)"'",
    "global_updated_at": "'"$(date -u +%Y-%m-%dT%H:%M:%S.000Z)"'"
  }]'
```

### List Tasks

```bash
curl -X GET "https://api.akiflow.com/v5/tasks?limit=100" \
  -H "Authorization: Bearer <token>" \
  -H "Akiflow-Client-Id: <uuid>" \
  -H "Akiflow-Version: 2.66.3" \
  -H "Akiflow-Platform: mac" \
  -H "Accept: application/json"
```

---

## Notes

1. **Upsert Pattern**: Akiflow uses PATCH with arrays for all create/update/delete operations. There's no POST endpoint for tasks.

2. **Soft Delete**: Entities are soft-deleted by setting `deleted_at`. They can be restored by setting it back to `null`.

3. **Client-Side Timestamps**: The client is responsible for generating `global_created_at`, `global_updated_at`, and `id` (UUID).

4. **Sync Token Validity**: Sync tokens may expire. If a sync request fails, fall back to full sync without token.

5. **Rate Limits**: Not officially documented. Standard practice: 100-200 requests/minute should be safe.

---

## Other Endpoints (Discovered)

| Endpoint | Description |
|----------|-------------|
| `/v5/accounts` | Connected accounts (Google, Notion, Slack, etc.) |
| `/v5/contacts` | Contact list |
| `/v3/notion/sync-now` | Trigger Notion sync |
| `/web/api/v2/user` | Web user info |
| `/web/api/pusherAuth` | Pusher authentication for real-time updates |
| `/aki/api/v1/chat/token` | AI chat token |

---

*Generated by reverse engineering Akiflow web app (v2.66.3)*
