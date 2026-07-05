import { invoke } from "@tauri-apps/api/core";

export type ManagedAuthProvider = "github_copilot" | "codex_oauth";

export interface ManagedAuthAccount {
  id: string;
  provider: ManagedAuthProvider;
  login: string;
  avatar_url: string | null;
  authenticated_at: number;
  is_default: boolean;
  github_domain: string;
}

export interface ManagedAuthStatus {
  provider: ManagedAuthProvider;
  authenticated: boolean;
  default_account_id: string | null;
  current_account_id: string | null;
  current_account_login: string | null;
  migration_error?: string | null;
  accounts: ManagedAuthAccount[];
}

export interface ManagedAuthDeviceCodeResponse {
  provider: ManagedAuthProvider;
  device_code: string;
  user_code: string;
  verification_uri: string;
  expires_in: number;
  interval: number;
}

export async function authStartLogin(
  authProvider: ManagedAuthProvider,
  githubDomain?: string,
): Promise<ManagedAuthDeviceCodeResponse> {
  return invoke<ManagedAuthDeviceCodeResponse>("auth_start_login", {
    authProvider,
    githubDomain: githubDomain || null,
  });
}

export async function authPollForAccount(
  authProvider: ManagedAuthProvider,
  deviceCode: string,
  githubDomain?: string,
): Promise<ManagedAuthAccount | null> {
  return invoke<ManagedAuthAccount | null>("auth_poll_for_account", {
    authProvider,
    deviceCode,
    githubDomain: githubDomain || null,
  });
}

export async function authListAccounts(
  authProvider: ManagedAuthProvider,
): Promise<ManagedAuthAccount[]> {
  return invoke<ManagedAuthAccount[]>("auth_list_accounts", {
    authProvider,
  });
}

export async function authGetStatus(
  authProvider: ManagedAuthProvider,
): Promise<ManagedAuthStatus> {
  return invoke<ManagedAuthStatus>("auth_get_status", {
    authProvider,
  });
}

export async function authRemoveAccount(
  authProvider: ManagedAuthProvider,
  accountId: string,
): Promise<void> {
  return invoke("auth_remove_account", {
    authProvider,
    accountId,
  });
}

export async function authSetDefaultAccount(
  authProvider: ManagedAuthProvider,
  accountId: string,
): Promise<void> {
  return invoke("auth_set_default_account", {
    authProvider,
    accountId,
  });
}

export async function authImportCurrent(
  authProvider: ManagedAuthProvider,
): Promise<ManagedAuthAccount> {
  return invoke<ManagedAuthAccount>("auth_import_current", {
    authProvider,
  });
}

export async function authImportJsonAccount(
  authProvider: ManagedAuthProvider,
  jsonContent: string,
  displayName?: string | null,
): Promise<ManagedAuthAccount> {
  return invoke<ManagedAuthAccount>("auth_import_json_account", {
    authProvider,
    jsonContent,
    displayName: displayName || null,
  });
}

export async function authImportJsonAccountFile(
  authProvider: ManagedAuthProvider,
  filePath: string,
  displayName?: string | null,
): Promise<ManagedAuthAccount> {
  return invoke<ManagedAuthAccount>("auth_import_json_account_file", {
    authProvider,
    filePath,
    displayName: displayName || null,
  });
}

export async function authSwitchCurrentAccount(
  authProvider: ManagedAuthProvider,
  accountId?: string | null,
): Promise<ManagedAuthAccount> {
  return invoke<ManagedAuthAccount>("auth_switch_current_account", {
    authProvider,
    accountId: accountId || null,
  });
}

export async function authLogout(
  authProvider: ManagedAuthProvider,
): Promise<void> {
  return invoke("auth_logout", {
    authProvider,
  });
}

export const authApi = {
  authStartLogin,
  authPollForAccount,
  authListAccounts,
  authGetStatus,
  authRemoveAccount,
  authSetDefaultAccount,
  authImportCurrent,
  authImportJsonAccount,
  authImportJsonAccountFile,
  authSwitchCurrentAccount,
  authLogout,
};
