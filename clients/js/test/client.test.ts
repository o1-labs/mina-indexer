import { test } from "node:test";
import assert from "node:assert/strict";
import { MinaIndexerClient, GraphQLError } from "../src/index.ts";

/** A fake `fetch` that routes by URL suffix to canned responses. */
function mockFetch(routes: Record<string, { status?: number; body: unknown }>): typeof fetch {
  return (async (input: string | URL | Request) => {
    const url = typeof input === "string" ? input : input.toString();
    const key = Object.keys(routes).find((k) => url.endsWith(k));
    if (!key) throw new Error(`no mock for ${url}`);
    const { status = 200, body } = routes[key];
    return new Response(JSON.stringify(body), {
      status,
      headers: { "content-type": "application/json" },
    });
  }) as typeof fetch;
}

test("isReady is true only when /readyz says ready", async () => {
  const ready = new MinaIndexerClient("http://x", {
    fetch: mockFetch({ "/readyz": { body: { ready: true, status: "ready" } } }),
  });
  assert.equal(await ready.isReady(), true);

  const behind = new MinaIndexerClient("http://x", {
    fetch: mockFetch({
      "/readyz": { status: 503, body: { ready: false, status: "catching_up", tip_age_seconds: 99999 } },
    }),
  });
  assert.equal(await behind.isReady(), false);
  assert.equal((await behind.readyz()).status, "catching_up");
});

test("healthz reflects HTTP ok", async () => {
  const up = new MinaIndexerClient("http://x/", {
    fetch: mockFetch({ "/healthz": { body: { status: "ok" } } }),
  });
  assert.equal(await up.healthz(), true);
});

test("graphql returns data and throws on errors", async () => {
  const ok = new MinaIndexerClient("http://x", {
    fetch: mockFetch({ "/graphql": { body: { data: { accountsCount: 42 } } } }),
  });
  assert.equal(await ok.accountsCount(), 42);

  const bad = new MinaIndexerClient("http://x", {
    fetch: mockFetch({ "/graphql": { body: { errors: [{ message: "boom" }] } } }),
  });
  await assert.rejects(() => bad.tipHeight(), GraphQLError);
});
