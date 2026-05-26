/// <reference types="bun" />
import { describe, expect, it } from "bun:test";
import {
	BROWSER_PATHS,
	type BrowserType,
	getAllBrowserPaths,
	getInstalledBrowsers,
	isBrowserInstalled,
} from "../../lib/browser-paths";

describe("BROWSER_PATHS", () => {
	it("contains all required browser keys", () => {
		// given
		const expectedBrowsers: BrowserType[] = [
			"chrome",
			"arc",
			"brave",
			"edge",
			"safari",
		];

		// when
		const actualBrowsers = Object.keys(BROWSER_PATHS) as BrowserType[];

		// then
		expect(actualBrowsers.sort()).toEqual(expectedBrowsers.sort());
	});

	it("contains Arc browser path with Arc in the path", () => {
		// given
		const arcPath = BROWSER_PATHS.arc;

		// when
		const hasArc = arcPath.includes("Arc");

		// then
		expect(hasArc).toBe(true);
	});

	it("contains Chrome browser path with Chrome in the path", () => {
		// given
		const chromePath = BROWSER_PATHS.chrome;

		// when
		const hasChrome = chromePath.includes("Chrome");

		// then
		expect(hasChrome).toBe(true);
	});

	it("contains Brave browser path with Brave in the path", () => {
		// given
		const bravePath = BROWSER_PATHS.brave;

		// when
		const hasBrave = bravePath.includes("Brave");

		// then
		expect(hasBrave).toBe(true);
	});

	it("contains Edge browser path with Edge in the path", () => {
		// given
		const edgePath = BROWSER_PATHS.edge;

		// when
		const hasEdge = edgePath.includes("Edge");

		// then
		expect(hasEdge).toBe(true);
	});

	it("contains Safari browser path with Cookies in the path", () => {
		// given
		const safariPath = BROWSER_PATHS.safari;

		// when
		const hasCookies = safariPath.includes("Cookies");

		// then
		expect(hasCookies).toBe(true);
	});

	it("all paths contain home directory reference", () => {
		// given
		const paths = Object.values(BROWSER_PATHS);

		// when
		const allHaveHome = paths.every((path) => path.includes("/"));

		// then
		expect(allHaveHome).toBe(true);
	});
});

describe("getAllBrowserPaths", () => {
	it("returns array with 5 browser configurations", () => {
		// given
		const browserPaths = getAllBrowserPaths();

		// when
		const count = browserPaths.length;

		// then
		expect(count).toBe(5);
	});

	it("returns browsers with correct encryption methods", () => {
		// given
		const browserPaths = getAllBrowserPaths();

		// when
		const chromeConfig = browserPaths.find((b) => b.id === "chrome");
		const safariConfig = browserPaths.find((b) => b.id === "safari");

		// then
		expect(chromeConfig?.encryptionMethod).toBe("pbkdf2");
		expect(safariConfig?.encryptionMethod).toBe("binary");
	});

	it("returns browsers with display names", () => {
		// given
		const browserPaths = getAllBrowserPaths();

		// when
		const allHaveNames = browserPaths.every((b) => b.name.length > 0);

		// then
		expect(allHaveNames).toBe(true);
	});

	it("returns browsers with cookie paths", () => {
		// given
		const browserPaths = getAllBrowserPaths();

		// when
		const allHavePaths = browserPaths.every((b) => b.cookiePath.length > 0);

		// then
		expect(allHavePaths).toBe(true);
	});
});

describe("isBrowserInstalled", () => {
	it("returns boolean for valid browser type", () => {
		// given
		const browserType: BrowserType = "chrome";

		// when
		const result = isBrowserInstalled(browserType);

		// then
		expect(typeof result).toBe("boolean");
	});

	it("returns false for non-existent browser paths", () => {
		// given
		const browserType: BrowserType = "safari";

		// when
		const result = isBrowserInstalled(browserType);

		// then
		expect(typeof result).toBe("boolean");
	});
});

describe("getInstalledBrowsers", () => {
	it("returns array of browser types", () => {
		// given
		const installedBrowsers = getInstalledBrowsers();

		// when
		const allAreStrings = installedBrowsers.every((b) => typeof b === "string");

		// then
		expect(allAreStrings).toBe(true);
	});

	it("returns only valid browser types", () => {
		// given
		const validBrowsers: BrowserType[] = [
			"chrome",
			"arc",
			"brave",
			"edge",
			"safari",
		];
		const installedBrowsers = getInstalledBrowsers();

		// when
		const allValid = installedBrowsers.every((b) => validBrowsers.includes(b));

		// then
		expect(allValid).toBe(true);
	});
});
