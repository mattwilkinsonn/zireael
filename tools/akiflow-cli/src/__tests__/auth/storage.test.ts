/// <reference types="bun" />
import { afterEach, beforeEach, describe, expect, it } from "bun:test";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
	clearCredentials,
	loadCredentials,
	saveCredentials,
} from "../../lib/auth/storage";

let TEST_CONFIG_DIR: string;
let ORIGINAL_AF_CONFIG_DIR: string | undefined;

function getCredentialsPath(): string {
	return join(TEST_CONFIG_DIR, "credentials.json");
}

const originalPlatform = process.platform;

function mockNonMacOS(): void {
	Object.defineProperty(process, "platform", {
		value: "linux",
		configurable: true,
	});
}

function restorePlatform(): void {
	Object.defineProperty(process, "platform", {
		value: originalPlatform,
		configurable: true,
	});
}

describe("Credentials Storage", () => {
	beforeEach(async () => {
		ORIGINAL_AF_CONFIG_DIR = process.env.AF_CONFIG_DIR;
		TEST_CONFIG_DIR = join(tmpdir(), `af-test-${crypto.randomUUID()}`);
		process.env.AF_CONFIG_DIR = TEST_CONFIG_DIR;

		mockNonMacOS();
		await clearCredentials();
	});

	afterEach(async () => {
		await clearCredentials();
		restorePlatform();

		if (ORIGINAL_AF_CONFIG_DIR === undefined) {
			delete process.env.AF_CONFIG_DIR;
		} else {
			process.env.AF_CONFIG_DIR = ORIGINAL_AF_CONFIG_DIR;
		}
	});

	describe("saveCredentials", () => {
		it("saves credentials with provided token and auto-generated clientId", async () => {
			// given
			const token = "test-jwt-token-123";

			// when
			await saveCredentials(token);

			// then
			const loaded = await loadCredentials();
			expect(loaded).not.toBeNull();
			expect(loaded?.token).toBe(token);
			expect(loaded?.clientId).toBeDefined();
			expect(loaded?.clientId).toMatch(
				/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i,
			);
		});

		it("saves credentials with provided clientId", async () => {
			// given
			const token = "test-jwt-token-456";
			const clientId = "12345678-1234-1234-1234-123456789012";

			// when
			await saveCredentials(token, clientId);

			// then
			const loaded = await loadCredentials();
			expect(loaded?.clientId).toBe(clientId);
		});

		it("saves credentials with provided expiryTimestamp", async () => {
			// given
			const token = "test-jwt-token-789";
			const expiryTimestamp = 1704067200000;

			// when
			await saveCredentials(token, undefined, expiryTimestamp);

			// then
			const loaded = await loadCredentials();
			expect(loaded?.expiryTimestamp).toBe(expiryTimestamp);
		});

		it("saves all credentials together", async () => {
			// given
			const token = "test-jwt-token-full";
			const clientId = "87654321-4321-4321-4321-210987654321";
			const expiryTimestamp = 1704153600000;

			// when
			await saveCredentials(token, clientId, expiryTimestamp);

			// then
			const loaded = await loadCredentials();
			expect(loaded).toEqual({
				token,
				clientId,
				expiryTimestamp,
			});
		});

		it("overwrites existing credentials", async () => {
			// given
			const token1 = "first-token";
			const token2 = "second-token";
			await saveCredentials(token1);

			// when
			await saveCredentials(token2);

			// then
			const loaded = await loadCredentials();
			expect(loaded?.token).toBe(token2);
		});
	});

	describe("loadCredentials", () => {
		it("returns null when no credentials exist", async () => {
			// given
			await clearCredentials();

			// when
			const result = await loadCredentials();

			// then
			expect(result).toBeNull();
		});

		it("loads saved credentials", async () => {
			// given
			const token = "load-test-token";
			const clientId = "load-test-client-id";
			const expiryTimestamp = 1704240000000;
			await saveCredentials(token, clientId, expiryTimestamp);

			// when
			const loaded = await loadCredentials();

			// then
			expect(loaded).toEqual({
				token,
				clientId,
				expiryTimestamp,
			});
		});

		it("returns null when credentials file is corrupted", async () => {
			// given
			if (process.platform !== "darwin") {
				const credentialsPath = getCredentialsPath();
				await Bun.$`mkdir -p ${TEST_CONFIG_DIR}`;
				await Bun.write(credentialsPath, "{ invalid json");
			}

			// when
			const result = await loadCredentials();

			// then
			expect(result).toBeNull();
		});
	});

	describe("clearCredentials", () => {
		it("clears saved credentials", async () => {
			// given
			await saveCredentials("test-token", "test-client-id");
			let loaded = await loadCredentials();
			expect(loaded).not.toBeNull();

			// when
			await clearCredentials();

			// then
			loaded = await loadCredentials();
			expect(loaded).toBeNull();
		});

		it("does not throw when clearing non-existent credentials", async () => {
			// given
			await clearCredentials();

			// when & then
			expect(async () => {
				await clearCredentials();
			}).not.toThrow();
		});
	});

	describe("Credentials interface", () => {
		it("has correct structure", async () => {
			// given
			const token = "interface-test-token";
			const clientId = "interface-test-client";
			const expiryTimestamp = 1704326400000;

			// when
			await saveCredentials(token, clientId, expiryTimestamp);
			const loaded = await loadCredentials();

			// then
			expect(loaded).toBeDefined();
			expect(typeof loaded?.token).toBe("string");
			expect(typeof loaded?.clientId).toBe("string");
			expect(typeof loaded?.expiryTimestamp).toBe("number");
		});
	});
});
