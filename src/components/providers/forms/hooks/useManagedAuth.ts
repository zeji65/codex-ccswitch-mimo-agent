import { useState, useCallback, useRef, useEffect } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { authApi, settingsApi } from "@/lib/api";
import { copyText } from "@/lib/clipboard";
import type {
  ManagedAuthAccount,
  ManagedAuthProvider,
  ManagedAuthStatus,
  ManagedAuthDeviceCodeResponse,
} from "@/lib/api";

type PollingState = "idle" | "polling" | "success" | "error";

export function useManagedAuth(
  authProvider: ManagedAuthProvider,
  githubDomain?: string,
) {
  const queryClient = useQueryClient();
  const queryKey = ["managed-auth-status", authProvider] as const;

  const [pollingState, setPollingState] = useState<PollingState>("idle");
  const [deviceCode, setDeviceCode] =
    useState<ManagedAuthDeviceCodeResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [lastCompletedAccount, setLastCompletedAccount] =
    useState<ManagedAuthAccount | null>(null);

  const pollingIntervalRef = useRef<ReturnType<typeof setInterval> | null>(
    null,
  );
  const pollingTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const {
    data: authStatus,
    isLoading: isLoadingStatus,
    refetch: refetchStatus,
  } = useQuery<ManagedAuthStatus>({
    queryKey,
    queryFn: () => authApi.authGetStatus(authProvider),
    staleTime: 30000,
  });

  const stopPolling = useCallback(() => {
    if (pollingIntervalRef.current) {
      clearInterval(pollingIntervalRef.current);
      pollingIntervalRef.current = null;
    }
    if (pollingTimeoutRef.current) {
      clearTimeout(pollingTimeoutRef.current);
      pollingTimeoutRef.current = null;
    }
  }, []);

  useEffect(() => {
    return () => {
      stopPolling();
    };
  }, [stopPolling]);

  const refreshManagedAuthState = useCallback(
    async (options?: { includeCodexProviders?: boolean }) => {
      await refetchStatus();
      await queryClient.invalidateQueries({ queryKey });

      if (authProvider !== "codex_oauth") {
        return;
      }

      if (options?.includeCodexProviders) {
        await queryClient.invalidateQueries({
          queryKey: ["providers", "codex"],
        });
      }

      await queryClient.invalidateQueries({
        queryKey: ["codex_oauth", "quota"],
      });
    },
    [authProvider, queryClient, queryKey, refetchStatus],
  );

  const startLoginMutation = useMutation({
    mutationFn: () => authApi.authStartLogin(authProvider, githubDomain),
    onSuccess: async (response) => {
      setDeviceCode(response);
      setPollingState("polling");
      setError(null);
      setLastCompletedAccount(null);

      try {
        await copyText(response.user_code);
      } catch (e) {
        console.debug("[ManagedAuth] Failed to copy user code:", e);
      }

      try {
        await settingsApi.openExternal(response.verification_uri);
      } catch (e) {
        console.debug("[ManagedAuth] Failed to open browser:", e);
      }

      // Add a small buffer on top of GitHub's suggested interval to avoid
      // hitting slow_down responses too aggressively during device polling.
      const interval = Math.max((response.interval || 5) + 3, 8) * 1000;
      const expiresAt = Date.now() + response.expires_in * 1000;

      const pollOnce = async () => {
        if (Date.now() > expiresAt) {
          stopPolling();
          setPollingState("error");
          setError("Device code expired. Please try again.");
          return;
        }

        try {
          const newAccount = await authApi.authPollForAccount(
            authProvider,
            response.device_code,
            githubDomain,
          );
          if (newAccount) {
            stopPolling();
            setLastCompletedAccount(newAccount);
            setPollingState("success");
            await refetchStatus();
            await queryClient.invalidateQueries({ queryKey });
            setPollingState("idle");
            setDeviceCode(null);
          }
        } catch (e) {
          const errorMessage = e instanceof Error ? e.message : String(e);
          if (
            !errorMessage.includes("pending") &&
            !errorMessage.includes("slow_down")
          ) {
            stopPolling();
            setPollingState("error");
            setError(errorMessage);
          }
        }
      };

      void pollOnce();
      pollingIntervalRef.current = setInterval(pollOnce, interval);
      pollingTimeoutRef.current = setTimeout(() => {
        stopPolling();
        setPollingState("error");
        setError("Device code expired. Please try again.");
      }, response.expires_in * 1000);
    },
    onError: (e) => {
      setPollingState("error");
      setError(e instanceof Error ? e.message : String(e));
      setLastCompletedAccount(null);
    },
  });

  const logoutMutation = useMutation({
    mutationFn: () => authApi.authLogout(authProvider),
    onSuccess: async () => {
      setPollingState("idle");
      setDeviceCode(null);
      setError(null);
      queryClient.setQueryData(queryKey, {
        provider: authProvider,
        authenticated: false,
        default_account_id: null,
        current_account_id: null,
        current_account_login: null,
        accounts: [],
      });
      await refreshManagedAuthState();
    },
    onError: async (e) => {
      console.error("[ManagedAuth] Failed to logout:", e);
      setError(e instanceof Error ? e.message : String(e));
      await refetchStatus();
    },
  });

  const removeAccountMutation = useMutation({
    mutationFn: (accountId: string) =>
      authApi.authRemoveAccount(authProvider, accountId),
    onSuccess: async () => {
      setPollingState("idle");
      setDeviceCode(null);
      setError(null);
      await refreshManagedAuthState();
    },
    onError: (e) => {
      console.error("[ManagedAuth] Failed to remove account:", e);
      setError(e instanceof Error ? e.message : String(e));
    },
  });

  const setDefaultAccountMutation = useMutation({
    mutationFn: (accountId: string) =>
      authApi.authSetDefaultAccount(authProvider, accountId),
    onSuccess: async () => {
      await refreshManagedAuthState();
    },
    onError: (e) => {
      console.error("[ManagedAuth] Failed to set default account:", e);
      setError(e instanceof Error ? e.message : String(e));
    },
  });

  const importCurrentMutation = useMutation({
    mutationFn: () => authApi.authImportCurrent(authProvider),
    onSuccess: async () => {
      setPollingState("idle");
      setDeviceCode(null);
      setError(null);
      await refreshManagedAuthState();
    },
    onError: (e) => {
      console.error("[ManagedAuth] Failed to import current auth:", e);
      setError(e instanceof Error ? e.message : String(e));
    },
  });

  const switchCurrentAccountMutation = useMutation({
    mutationFn: (accountId: string | null) =>
      authApi.authSwitchCurrentAccount(authProvider, accountId),
    onSuccess: async () => {
      setError(null);
      await refreshManagedAuthState({ includeCodexProviders: true });
    },
    onError: (e) => {
      console.error("[ManagedAuth] Failed to switch current account:", e);
      setError(e instanceof Error ? e.message : String(e));
    },
  });

  const rotateCodexAccountMutation = useMutation({
    mutationFn: (forceNext?: boolean) => {
      if (authProvider !== "codex_oauth") {
        return Promise.reject(new Error("当前仅支持 Codex OAuth 自动轮换账号"));
      }
      return authApi.rotateCodexAccount(forceNext ?? false);
    },
    onSuccess: async () => {
      setError(null);
      await refreshManagedAuthState({ includeCodexProviders: true });
    },
    onError: (e) => {
      console.error("[ManagedAuth] Failed to rotate Codex account:", e);
      setError(e instanceof Error ? e.message : String(e));
    },
  });

  const startAuth = useCallback(async () => {
    setPollingState("idle");
    setDeviceCode(null);
    setError(null);
    setLastCompletedAccount(null);
    stopPolling();
    return startLoginMutation.mutateAsync();
  }, [startLoginMutation, stopPolling]);

  const cancelAuth = useCallback(() => {
    stopPolling();
    setPollingState("idle");
    setDeviceCode(null);
    setError(null);
    setLastCompletedAccount(null);
  }, [stopPolling]);

  const logout = useCallback(() => {
    logoutMutation.mutate();
  }, [logoutMutation]);

  const removeAccount = useCallback(
    (accountId: string) => {
      removeAccountMutation.mutate(accountId);
    },
    [removeAccountMutation],
  );

  const setDefaultAccount = useCallback(
    (accountId: string) => {
      setDefaultAccountMutation.mutate(accountId);
    },
    [setDefaultAccountMutation],
  );

  const importCurrent = useCallback(async () => {
    setError(null);
    return importCurrentMutation.mutateAsync();
  }, [importCurrentMutation]);

  const switchCurrentAccount = useCallback(
    async (accountId: string | null) => {
      setError(null);
      return switchCurrentAccountMutation.mutateAsync(accountId);
    },
    [switchCurrentAccountMutation],
  );

  const rotateCodexAccount = useCallback(
    async (forceNext = false) => {
      setError(null);
      return rotateCodexAccountMutation.mutateAsync(forceNext);
    },
    [rotateCodexAccountMutation],
  );

  const accounts = authStatus?.accounts ?? [];

  return {
    authStatus,
    isLoadingStatus,
    accounts,
    hasAnyAccount: accounts.length > 0,
    isAuthenticated: authStatus?.authenticated ?? false,
    defaultAccountId: authStatus?.default_account_id ?? null,
    currentAccountId: authStatus?.current_account_id ?? null,
    currentAccountLogin: authStatus?.current_account_login ?? null,
    migrationError: authStatus?.migration_error ?? null,
    pollingState,
    deviceCode,
    error,
    lastCompletedAccount,
    isPolling: pollingState === "polling",
    isAddingAccount: startLoginMutation.isPending || pollingState === "polling",
    isRemovingAccount: removeAccountMutation.isPending,
    isSettingDefaultAccount: setDefaultAccountMutation.isPending,
    isImportingCurrent: importCurrentMutation.isPending,
    isSwitchingCurrent: switchCurrentAccountMutation.isPending,
    isRotatingCodex: rotateCodexAccountMutation.isPending,
    startAuth,
    addAccount: startAuth,
    importCurrent,
    switchCurrentAccount,
    rotateCodexAccount,
    cancelAuth,
    logout,
    removeAccount,
    setDefaultAccount,
    refetchStatus,
  };
}
