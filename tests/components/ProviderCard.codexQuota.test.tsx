import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ProviderCard } from "@/components/providers/ProviderCard";
import type { Provider } from "@/types";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { defaultValue?: string }) =>
      options?.defaultValue ?? key,
  }),
}));

vi.mock("@/components/providers/ProviderActions", () => ({
  ProviderActions: () => <div data-testid="provider-actions" />,
}));

vi.mock("@/components/ProviderIcon", () => ({
  ProviderIcon: () => <div data-testid="provider-icon" />,
}));

vi.mock("@/components/UsageFooter", () => ({
  default: () => <div data-testid="usage-footer" />,
}));

vi.mock("@/components/SubscriptionQuotaFooter", () => ({
  default: () => <div data-testid="subscription-footer" />,
}));

vi.mock("@/components/CopilotQuotaFooter", () => ({
  default: () => <div data-testid="copilot-footer" />,
}));

vi.mock("@/components/CodexOauthQuotaFooter", () => ({
  default: () => <div data-testid="codex-oauth-footer" />,
}));

vi.mock("@/components/providers/ProviderHealthBadge", () => ({
  ProviderHealthBadge: () => null,
}));

vi.mock("@/components/providers/FailoverPriorityBadge", () => ({
  FailoverPriorityBadge: () => null,
}));

vi.mock("@/config/hermesProviderPresets", () => ({
  isHermesReadOnlyProvider: () => false,
}));

vi.mock("@/utils/providerConfigUtils", () => ({
  extractCodexBaseUrl: () => null,
}));

vi.mock("@/lib/query/failover", () => ({
  useProviderHealth: () => ({ data: null }),
}));

vi.mock("@/lib/query/queries", () => ({
  useUsageQuery: () => ({ data: undefined }),
}));

function createProvider(overrides: Partial<Provider> = {}): Provider {
  return {
    id: overrides.id ?? "provider-1",
    name: overrides.name ?? "Test Provider",
    settingsConfig: overrides.settingsConfig ?? {},
    category: overrides.category ?? "custom",
    createdAt: overrides.createdAt ?? Date.now(),
    sortIndex: overrides.sortIndex ?? 0,
    notes: overrides.notes,
    meta: overrides.meta,
    websiteUrl: overrides.websiteUrl,
  };
}

describe("ProviderCard codex quota footer selection", () => {
  it("renders codex oauth quota for a managed codex account even when provider metadata is incomplete", () => {
    const provider = createProvider({
      category: "custom",
      settingsConfig: {},
      meta: {
        authBinding: {
          source: "managed_account",
          authProvider: "codex_oauth",
          accountId: "account-123",
        },
      },
    });

    render(
      <ProviderCard
        provider={provider}
        isCurrent={false}
        appId="codex"
        onSwitch={vi.fn()}
        onEdit={vi.fn()}
        onDelete={vi.fn()}
        onConfigureUsage={vi.fn()}
        onOpenWebsite={vi.fn()}
        onDuplicate={vi.fn()}
        isProxyRunning={false}
      />,
    );

    expect(screen.getByTestId("codex-oauth-footer")).toBeInTheDocument();
    expect(screen.queryByTestId("subscription-footer")).not.toBeInTheDocument();
  });
});
