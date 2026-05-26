import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { FakeAkiflowServer } from "./fake-server";

let server: FakeAkiflowServer;
beforeEach(() => {
	server = new FakeAkiflowServer();
});
afterEach(async () => {
	await server.stop();
});

describe("FakeAkiflowServer", () => {
	test("starts on a random port and returns its URL", async () => {
		const url = await server.start();
		expect(url).toMatch(/^http:\/\/127\.0\.0\.1:\d+$/);
	});

	test("returns canned response for a registered endpoint", async () => {
		await server.start();
		server.respondTo("GET", "/v5/tasks", {
			success: true,
			message: null,
			data: [{ id: "a" }],
			sync_token: "tok1",
			has_next_page: false,
		});
		const resp = await fetch(`${server.url}/v5/tasks?limit=50`);
		const body = (await resp.json()) as { data: unknown[]; sync_token: string };
		expect(body.data).toEqual([{ id: "a" }]);
		expect(body.sync_token).toBe("tok1");
	});

	test("returns 404 when no canned response", async () => {
		await server.start();
		const resp = await fetch(`${server.url}/v5/tasks`);
		expect(resp.status).toBe(404);
	});

	test("records all requests for assertion", async () => {
		await server.start();
		server.respondTo("GET", "/v5/tasks", {
			success: true,
			message: null,
			data: [],
			sync_token: "tok",
			has_next_page: false,
		});
		await fetch(`${server.url}/v5/tasks?limit=50&sync_token=abc`);
		expect(server.requests).toHaveLength(1);
		expect(server.requests[0]?.url.searchParams.get("limit")).toBe("50");
		expect(server.requests[0]?.url.searchParams.get("sync_token")).toBe("abc");
	});
});
