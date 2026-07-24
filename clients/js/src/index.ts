/**
 * TypeScript client for the {@link https://github.com/o1-labs/mina-indexer Mina Indexer}
 * REST + GraphQL API.
 *
 * The indexer serves REST (`/summary`, `/healthz`, `/readyz`) and GraphQL
 * (`/graphql`) on one port. This client wraps both and leads with the
 * health / sync surface, so a caller can refuse to trust an indexer that is
 * still catching up.
 *
 * ```ts
 * import { MinaIndexerClient } from "@o1-labs/mina-indexer-client";
 *
 * const client = new MinaIndexerClient("https://devnet-indexer.gcp.o1test.net");
 *
 * if (!(await client.isReady())) {
 *   console.warn("indexer is catching up — not querying");
 * } else {
 *   console.log("tip", await client.tipHeight());
 *   console.log("accounts", await client.accountsCount());
 * }
 * ```
 *
 * Works anywhere the global `fetch` exists (Node ≥ 18, Deno, browsers).
 */

/** `/readyz` response — whether the indexer's tip is fresh enough to trust. */
export interface Readiness {
  /** `true` when the best tip is within the indexer's lag budget. */
  ready: boolean;
  /** `ready` | `catching_up` | `bootstrapping` | `store_unavailable`. */
  status: string;
  tip_height?: number;
  tip_age_seconds?: number;
  max_lag_seconds?: number;
}

export class MinaIndexerError extends Error {}
export class GraphQLError extends MinaIndexerError {
  constructor(public errors: unknown) {
    super(`graphql errors: ${JSON.stringify(errors)}`);
  }
}

export interface ClientOptions {
  /** Custom fetch (e.g. with a timeout/agent). Defaults to global `fetch`. */
  fetch?: typeof fetch;
}

export class MinaIndexerClient {
  private readonly baseUrl: string;
  private readonly doFetch: typeof fetch;

  /** @param baseUrl e.g. `http://localhost:8080` (a trailing slash is trimmed). */
  constructor(baseUrl: string, opts: ClientOptions = {}) {
    this.baseUrl = baseUrl.replace(/\/+$/, "");
    const f = opts.fetch ?? globalThis.fetch;
    if (!f) throw new MinaIndexerError("no fetch available; pass opts.fetch");
    this.doFetch = f;
  }

  // ---- health / sync ----

  /** Liveness (`GET /healthz`): `true` if the process is up and the store answers. */
  async healthz(): Promise<boolean> {
    const res = await this.doFetch(this.url("/healthz"));
    return res.ok;
  }

  /** Readiness (`GET /readyz`): the full status object (body is present on 503 too). */
  async readyz(): Promise<Readiness> {
    const res = await this.doFetch(this.url("/readyz"));
    return (await res.json()) as Readiness;
  }

  /** `true` only when the indexer reports itself ready (tip fresh). Gate queries on this. */
  async isReady(): Promise<boolean> {
    return (await this.readyz()).ready === true;
  }

  // ---- REST ----

  /** `GET /summary` — the blockchain summary object. */
  async summary(): Promise<Record<string, unknown>> {
    const res = await this.doFetch(this.url("/summary"));
    if (!res.ok) throw new MinaIndexerError(`GET /summary -> ${res.status}`);
    return (await res.json()) as Record<string, unknown>;
  }

  /** The indexer's store schema version, e.g. `0.19.0-<git>` (from `/summary`). */
  async dbVersion(): Promise<string> {
    const s = await this.summary();
    const v = s["dbVersion"];
    if (typeof v !== "string") throw new MinaIndexerError("summary missing dbVersion");
    return v;
  }

  // ---- GraphQL ----

  /**
   * Run a GraphQL query and return its `data`, typed as `T`. Throws
   * {@link GraphQLError} if the response carries `errors`.
   */
  async graphql<T = unknown>(query: string, variables: Record<string, unknown> = {}): Promise<T> {
    const res = await this.doFetch(this.url("/graphql"), {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ query, variables }),
    });
    if (!res.ok) throw new MinaIndexerError(`POST /graphql -> ${res.status}`);
    const body = (await res.json()) as { data?: T; errors?: unknown };
    if (body.errors) throw new GraphQLError(body.errors);
    if (body.data === undefined) throw new MinaIndexerError("graphql response had no data");
    return body.data;
  }

  // ---- typed convenience queries ----

  /** Height of the canonical best tip. */
  async tipHeight(): Promise<number> {
    const data = await this.graphql<{ blocks: { blockHeight: number }[] }>(
      "{ blocks(limit:1, sortBy: BLOCKHEIGHT_DESC, query:{canonical:true}) { blockHeight } }",
    );
    const h = data.blocks[0]?.blockHeight;
    if (h === undefined) throw new MinaIndexerError("no best block");
    return h;
  }

  /**
   * Total accounts matching `query` (the whole ledger when omitted). Pass a
   * GraphQL `AccountQueryInput` literal, e.g. `"{ balance_gte: 0 }"`.
   */
  async accountsCount(query?: string): Promise<number> {
    const q = query ? `{ accountsCount(query: ${query}) }` : "{ accountsCount }";
    const data = await this.graphql<{ accountsCount: number }>(q);
    return data.accountsCount;
  }

  private url(path: string): string {
    return `${this.baseUrl}${path}`;
  }
}
