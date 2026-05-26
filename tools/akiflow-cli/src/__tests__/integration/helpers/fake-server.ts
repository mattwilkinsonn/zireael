interface RecordedRequest {
	method: string;
	url: URL;
	headers: Record<string, string>;
	body: string;
}

type ResponseValue = unknown | ((req: { body: string; url: URL }) => unknown);

interface Responder {
	method: string;
	path: string;
	response: ResponseValue;
	status?: number;
}

/**
 * In-memory HTTP server for integration tests. Listens on a random port,
 * matches incoming requests against responders registered via respondTo(),
 * records every request for assertion.
 *
 * Start with `await server.start()`, point the CLI at it via the
 * `AF_API_BASE` env var (the value of `server.url`), stop with
 * `await server.stop()` in afterEach.
 */
export class FakeAkiflowServer {
	private server: ReturnType<typeof Bun.serve> | null = null;
	private responders: Responder[] = [];
	public readonly requests: RecordedRequest[] = [];
	public url = "";

	async start(): Promise<string> {
		this.server = Bun.serve({
			port: 0,
			hostname: "127.0.0.1",
			fetch: async (req) => {
				const url = new URL(req.url);
				const body = req.body ? await req.text() : "";
				const headers: Record<string, string> = {};
				req.headers.forEach((v, k) => {
					headers[k] = v;
				});
				this.requests.push({ method: req.method, url, headers, body });

				const r = this.responders.find(
					(r) => r.method === req.method && r.path === url.pathname,
				);
				if (!r) return new Response("Not Found", { status: 404 });
				const value =
					typeof r.response === "function"
						? (r.response as (req: { body: string; url: URL }) => unknown)({
								body,
								url,
							})
						: r.response;
				return new Response(JSON.stringify(value), {
					status: r.status ?? 200,
					headers: { "content-type": "application/json" },
				});
			},
		});
		this.url = `http://127.0.0.1:${this.server.port}`;
		return this.url;
	}

	async stop(): Promise<void> {
		if (this.server) this.server.stop(true);
		this.server = null;
	}

	respondTo(
		method: string,
		path: string,
		response: ResponseValue,
		status?: number,
	): void {
		this.responders.push({ method, path, response, status });
	}

	reset(): void {
		this.responders.length = 0;
		this.requests.length = 0;
	}
}
