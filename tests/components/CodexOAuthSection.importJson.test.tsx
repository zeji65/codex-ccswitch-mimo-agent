import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CodexOAuthSection } from "@/components/providers/forms/CodexOAuthSection";

const mocks = vi.hoisted(() => ({
  importJsonAccount: vi.fn(),
  importJsonAccountFromFile: vi.fn(),
  openJsonFileDialog: vi.fn(),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, fallback?: string | { defaultValue?: string }) =>
      typeof fallback === "string"
        ? fallback
        : (fallback?.defaultValue ?? _key),
  }),
}));

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn(),
    error: vi.fn(),
  },
}));

vi.mock("@/lib/api", () => ({
  settingsApi: {
    openJsonFileDialog: mocks.openJsonFileDialog,
  },
}));

vi.mock("@/components/providers/forms/hooks/useCodexOauth", () => ({
  useCodexOauth: () => ({
    accounts: [],
    defaultAccountId: null,
    hasAnyAccount: false,
    pollingState: "idle",
    deviceCode: null,
    error: null,
    isPolling: false,
    isAddingAccount: false,
    isRemovingAccount: false,
    isSettingDefaultAccount: false,
    isImportingCurrentAccount: false,
    isImportingJsonAccount: false,
    addAccount: vi.fn(),
    removeAccount: vi.fn(),
    setDefaultAccount: vi.fn(),
    cancelAuth: vi.fn(),
    logout: vi.fn(),
    importCurrentAccount: vi.fn(),
    importJsonAccount: mocks.importJsonAccount,
    importJsonAccountFromFile: mocks.importJsonAccountFromFile,
  }),
}));

describe("CodexOAuthSection JSON import", () => {
  beforeEach(() => {
    mocks.importJsonAccount.mockReset();
    mocks.importJsonAccountFromFile.mockReset();
    mocks.openJsonFileDialog.mockReset();
    mocks.importJsonAccount.mockResolvedValue({
      id: "acc-123",
      login: "企业号",
    });
    mocks.importJsonAccountFromFile.mockResolvedValue({
      id: "acc-123",
      login: "企业号",
    });
  });

  it("opens the native JSON file picker directly and imports the selected account", async () => {
    const user = userEvent.setup();
    mocks.openJsonFileDialog.mockResolvedValue("/tmp/accounts/001.json");

    render(<CodexOAuthSection />);

    await user.click(screen.getByRole("button", { name: "导入 JSON 账号" }));

    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(mocks.openJsonFileDialog).toHaveBeenCalledOnce();

    await waitFor(() =>
      expect(mocks.importJsonAccountFromFile).toHaveBeenCalledWith(
        "/tmp/accounts/001.json",
        null,
      ),
    );
  });

  it("keeps the page usable when JSON file selection is cancelled", async () => {
    const user = userEvent.setup();
    mocks.openJsonFileDialog.mockResolvedValue(null);

    render(<CodexOAuthSection />);

    await user.click(screen.getByRole("button", { name: "导入 JSON 账号" }));

    await waitFor(() => expect(mocks.openJsonFileDialog).toHaveBeenCalledOnce());
    expect(mocks.importJsonAccountFromFile).not.toHaveBeenCalled();
    expect(
      screen.getByRole("button", { name: "导入 JSON 账号" }),
    ).toBeEnabled();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
