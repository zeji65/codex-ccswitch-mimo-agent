import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import CodexOauthQuotaFooter from "@/components/CodexOauthQuotaFooter";
import { useCodexOauthQuota } from "@/lib/query/subscription";

vi.mock("@/lib/query/subscription", () => ({
  useCodexOauthQuota: vi.fn(() => ({
    data: undefined,
    isFetching: false,
    refetch: vi.fn(),
  })),
}));

vi.mock("@/components/SubscriptionQuotaFooter", () => ({
  SubscriptionQuotaView: () => <div data-testid="quota-view" />,
}));

describe("CodexOauthQuotaFooter", () => {
  it("enables auto query even when the provider is not current", () => {
    render(<CodexOauthQuotaFooter isCurrent={false} />);

    expect(useCodexOauthQuota).toHaveBeenCalledWith(undefined, {
      enabled: true,
      autoQuery: true,
    });
  });
});
