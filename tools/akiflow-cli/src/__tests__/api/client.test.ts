/// <reference types="bun" />
import { afterEach, beforeEach, describe, expect, it, spyOn } from "bun:test";
import { AkiflowClient, createClient } from "../../lib/api/client";
import { AuthError, NetworkError } from "../../lib/api/types";
import * as storage from "../../lib/auth/storage";

const mockCredentials = {
	token: "test-jwt-token",
	clientId: "test-client-id-12345",
	expiryTimestamp: Date.now() + 86400000,
};

const mockTaskResponse = {
	success: true,
	message: null,
	data: [
		{
			id: "task-123",
			user_id: 1,
			title: "Test Task",
			done: false,
			status: 0,
			tags_ids: [],
			links: [],
			doc: {},
			content: {},
			data: {},
			sorting: 0,
			global_created_at: "2026-02-01T00:00:00.000Z",
			global_updated_at: "2026-02-01T00:00:00.000Z",
		},
	],
	sync_token: "test-sync-token",
	has_next_page: false,
};

describe("AkiflowClient", () => {
	let fetchSpy: ReturnType<typeof spyOn>;
	let loadCredentialsSpy: ReturnType<typeof spyOn>;

	beforeEach(() => {
		loadCredentialsSpy = spyOn(storage, "loadCredentials").mockResolvedValue(
			mockCredentials,
		);
	});

	afterEach(() => {
		fetchSpy?.mockRestore();
		loadCredentialsSpy?.mockRestore();
	});

	describe("constructor", () => {
		it("creates client with default options", () => {
			// given & when
			const client = new AkiflowClient();

			// then
			expect(client).toBeInstanceOf(AkiflowClient);
		});

		it("creates client with provided credentials", async () => {
			// given
			const credentials = { token: "custom-token", clientId: "custom-client" };
			fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(
				new Response(JSON.stringify(mockTaskResponse), { status: 200 }),
			);

			// when
			const client = new AkiflowClient({ credentials });
			await client.getTasks();

			// then
			expect(fetchSpy).toHaveBeenCalledWith(
				expect.stringContaining("/v5/tasks"),
				expect.objectContaining({
					headers: expect.objectContaining({
						Authorization: "Bearer custom-token",
						"Akiflow-Client-Id": "custom-client",
					}),
				}),
			);
		});
	});

	describe("getTasks", () => {
		it("fetches tasks with default limit", async () => {
			// given
			fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(
				new Response(JSON.stringify(mockTaskResponse), { status: 200 }),
			);
			const client = createClient();

			// when
			const result = await client.getTasks();

			// then
			expect(fetchSpy).toHaveBeenCalledWith(
				"https://api.akiflow.com/v5/tasks?limit=2500",
				expect.objectContaining({
					method: "GET",
					headers: expect.objectContaining({
						Authorization: `Bearer ${mockCredentials.token}`,
						"Akiflow-Client-Id": mockCredentials.clientId,
						"Akiflow-Version": "3",
						"Akiflow-Platform": "web",
						Accept: "application/json",
					}),
				}),
			);
			expect(result.success).toBe(true);
			expect(result.data).toHaveLength(1);
		});

		it("fetches tasks with custom limit and sync token", async () => {
			// given
			fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(
				new Response(JSON.stringify(mockTaskResponse), { status: 200 }),
			);
			const client = createClient();

			// when
			await client.getTasks({ limit: 100, syncToken: "prev-sync-token" });

			// then
			expect(fetchSpy).toHaveBeenCalledWith(
				"https://api.akiflow.com/v5/tasks?limit=100&sync_token=prev-sync-token",
				expect.any(Object),
			);
		});

		it("throws AuthError on 401 response", async () => {
			// given
			fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(
				new Response("Unauthorized", { status: 401 }),
			);
			const client = createClient();

			// when & then
			await expect(client.getTasks()).rejects.toThrow(AuthError);
		});

		it("throws NetworkError on network failure", async () => {
			// given
			fetchSpy = spyOn(globalThis, "fetch").mockRejectedValue(
				new Error("Network error"),
			);
			const client = createClient();

			// when & then
			await expect(client.getTasks()).rejects.toThrow(NetworkError);
		});

		it("throws NetworkError on non-ok response", async () => {
			// given
			fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(
				new Response("Server Error", {
					status: 500,
					statusText: "Internal Server Error",
				}),
			);
			const client = createClient();

			// when & then
			await expect(client.getTasks()).rejects.toThrow(NetworkError);
		});
	});

	describe("getAllTasks", () => {
		it("paginates using sync_token cursor", async () => {
			// given
			const page1 = {
				...mockTaskResponse,
				data: [
					{
						...mockTaskResponse.data[0]!,
						id: "task-1",
						title: "Page 1 Task",
					},
				],
				sync_token: "token-1",
				has_next_page: true,
			};

			const page2 = {
				...mockTaskResponse,
				data: [
					{
						...mockTaskResponse.data[0]!,
						id: "task-2",
						title: "Page 2 Task",
					},
				],
				sync_token: "token-2",
				has_next_page: false,
			};

			fetchSpy = spyOn(globalThis, "fetch")
				.mockResolvedValueOnce(
					new Response(JSON.stringify(page1), { status: 200 }),
				)
				.mockResolvedValueOnce(
					new Response(JSON.stringify(page2), { status: 200 }),
				);

			const client = createClient();

			// when
			const all = await client.getAllTasks();

			// then
			expect(all).toHaveLength(2);
			expect(all.map((t) => t.id)).toEqual(["task-1", "task-2"]);

			expect(fetchSpy).toHaveBeenNthCalledWith(
				1,
				"https://api.akiflow.com/v5/tasks?limit=2500",
				expect.any(Object),
			);

			expect(fetchSpy).toHaveBeenNthCalledWith(
				2,
				"https://api.akiflow.com/v5/tasks?limit=2500&sync_token=token-1",
				expect.any(Object),
			);
		});
	});

	describe("upsertTasks", () => {
		it("sends PATCH request with task array", async () => {
			// given
			fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(
				new Response(JSON.stringify(mockTaskResponse), { status: 200 }),
			);
			const client = createClient();
			const taskPayload = {
				id: "new-task-id",
				title: "New Task",
				global_created_at: "2026-02-01T00:00:00.000Z",
				global_updated_at: "2026-02-01T00:00:00.000Z",
			};

			// when
			await client.upsertTasks([taskPayload]);

			// then
			expect(fetchSpy).toHaveBeenCalledWith(
				"https://api.akiflow.com/v5/tasks",
				expect.objectContaining({
					method: "PATCH",
					headers: expect.objectContaining({
						"Content-Type": "application/json",
					}),
					body: JSON.stringify([taskPayload]),
				}),
			);
		});
	});

	describe("getLabels", () => {
		it("fetches labels from correct endpoint", async () => {
			// given
			const mockResponse = { success: true, message: null, data: [] };
			fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(
				new Response(JSON.stringify(mockResponse), { status: 200 }),
			);
			const client = createClient();

			// when
			await client.getLabels();

			// then
			expect(fetchSpy).toHaveBeenCalledWith(
				"https://api.akiflow.com/v5/labels?limit=2500",
				expect.any(Object),
			);
		});

		it("fetches labels with sync token", async () => {
			// given
			const mockResponse = { success: true, message: null, data: [] };
			fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(
				new Response(JSON.stringify(mockResponse), { status: 200 }),
			);
			const client = createClient();

			// when
			await client.getLabels({ syncToken: "label-sync-token" });

			// then
			expect(fetchSpy).toHaveBeenCalledWith(
				"https://api.akiflow.com/v5/labels?limit=2500&sync_token=label-sync-token",
				expect.any(Object),
			);
		});
	});

	describe("getTags", () => {
		it("fetches tags from correct endpoint", async () => {
			// given
			const mockResponse = { success: true, message: null, data: [] };
			fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(
				new Response(JSON.stringify(mockResponse), { status: 200 }),
			);
			const client = createClient();

			// when
			await client.getTags();

			// then
			expect(fetchSpy).toHaveBeenCalledWith(
				"https://api.akiflow.com/v5/tags?limit=2500",
				expect.any(Object),
			);
		});
	});

	describe("getTimeSlots", () => {
		it("fetches time slots from correct endpoint", async () => {
			// given
			const mockResponse = { success: true, message: null, data: [] };
			fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(
				new Response(JSON.stringify(mockResponse), { status: 200 }),
			);
			const client = createClient();

			// when
			await client.getTimeSlots();

			// then
			expect(fetchSpy).toHaveBeenCalledWith(
				"https://api.akiflow.com/v5/time_slots?limit=2500",
				expect.any(Object),
			);
		});
	});

	describe("credential loading", () => {
		it("throws AuthError when no credentials available", async () => {
			// given
			loadCredentialsSpy.mockResolvedValue(null);
			const client = createClient();

			// when & then
			await expect(client.getTasks()).rejects.toThrow(AuthError);
			await expect(client.getTasks()).rejects.toThrow(
				"No credentials found. Please login first.",
			);
		});

		it("uses stored credentials when not provided in constructor", async () => {
			// given
			fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(
				new Response(JSON.stringify(mockTaskResponse), { status: 200 }),
			);
			const client = createClient();

			// when
			await client.getTasks();

			// then
			expect(loadCredentialsSpy).toHaveBeenCalled();
			expect(fetchSpy).toHaveBeenCalledWith(
				expect.any(String),
				expect.objectContaining({
					headers: expect.objectContaining({
						Authorization: `Bearer ${mockCredentials.token}`,
					}),
				}),
			);
		});
	});

	describe("createClient helper", () => {
		it("returns AkiflowClient instance", () => {
			// given & when
			const client = createClient();

			// then
			expect(client).toBeInstanceOf(AkiflowClient);
		});

		it("passes options to constructor", async () => {
			// given
			fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(
				new Response(JSON.stringify(mockTaskResponse), { status: 200 }),
			);
			const customVersion = "2.66.3";
			const customPlatform = "mac";

			// when
			const client = createClient({
				version: customVersion,
				platform: customPlatform,
			});
			await client.getTasks();

			// then
			expect(fetchSpy).toHaveBeenCalledWith(
				expect.any(String),
				expect.objectContaining({
					headers: expect.objectContaining({
						"Akiflow-Version": customVersion,
						"Akiflow-Platform": customPlatform,
					}),
				}),
			);
		});
	});
});
