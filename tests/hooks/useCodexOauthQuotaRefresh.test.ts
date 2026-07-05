import { describe, expect, it } from "vitest";
import {
  CODEX_OAUTH_FAST_REFETCH_INTERVAL,
  CODEX_OAUTH_REFETCH_INTERVAL,
  getCodexOauthQuotaRefetchInterval,
} from "@/lib/query/subscription";
import type { SubscriptionQuota } from "@/types/subscription";

const quota = (
  overrides: Partial<SubscriptionQuota> = {},
): SubscriptionQuota => ({
  tool: "codex_oauth",
  credentialStatus: "valid",
  credentialMessage: null,
  success: true,
  tiers: [
    {
      name: "five_hour",
      utilization: 52,
      resetsAt: "2026-06-03T12:00:00Z",
    },
  ],
  extraUsage: null,
  error: null,
  queriedAt: Date.now(),
  ...overrides,
});

describe("Codex OAuth quota refresh interval", () => {
  it("keeps the normal interval after quota tiers are returned", () => {
    expect(getCodexOauthQuotaRefetchInterval(quota())).toBe(
      CODEX_OAUTH_REFETCH_INTERVAL,
    );
  });

  it("uses the fast interval when the quota request fails", () => {
    expect(
      getCodexOauthQuotaRefetchInterval(
        quota({ success: false, tiers: [], error: "Network error" }),
      ),
    ).toBe(CODEX_OAUTH_FAST_REFETCH_INTERVAL);
  });

  it("uses the fast interval when the quota response has no tiers", () => {
    expect(getCodexOauthQuotaRefetchInterval(quota({ tiers: [] }))).toBe(
      CODEX_OAUTH_FAST_REFETCH_INTERVAL,
    );
  });
});
