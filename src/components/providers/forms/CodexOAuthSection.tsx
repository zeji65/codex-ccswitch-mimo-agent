import React from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Loader2,
  LogOut,
  Copy,
  Check,
  ExternalLink,
  Plus,
  X,
  Sparkles,
  User,
  ArrowDownToLine,
  RefreshCw,
} from "lucide-react";
import { useCodexOauth } from "./hooks/useCodexOauth";
import { copyText } from "@/lib/clipboard";

interface CodexOAuthSectionProps {
  className?: string;
  /** 当前选中的 ChatGPT 账号 ID */
  selectedAccountId?: string | null;
  /** 账号选择回调 */
  onAccountSelect?: (accountId: string | null) => void;
  /** 是否允许直接切换当前生效的 Codex 官方账号 */
  enableDirectSwitch?: boolean;
}

/**
 * Codex OAuth 认证区块
 *
 * 通过 OpenAI Device Code 流程登录 ChatGPT Plus/Pro 账号，
 * 可用于 Claude 侧的 Codex 反代，也可用于 Codex 官方 provider 的原生账号切换。
 */
export const CodexOAuthSection: React.FC<CodexOAuthSectionProps> = ({
  className,
  selectedAccountId,
  onAccountSelect,
  enableDirectSwitch = false,
}) => {
  const { t } = useTranslation();
  const [copied, setCopied] = React.useState(false);
  const [reauthTarget, setReauthTarget] = React.useState<{
    id: string;
    login: string;
  } | null>(null);

  const stripCodexSwitchErrorPrefix = React.useCallback((message: string) => {
    return message
      .replace(/^CODEX_OAUTH_[A-Z_]+:\s*/, "")
      .replace(/^切换当前 Codex 官方账号失败:\s*/, "")
      .replace(/^切换前校验 Codex 官方账号失败:\s*/, "")
      .trim();
  }, []);

  const isRecoverableCodexSwitchError = React.useCallback((message: string) => {
    return [
      "CODEX_OAUTH_REAUTH_REQUIRED",
      "CODEX_OAUTH_IMPORT_CURRENT_REQUIRED",
      "CODEX_OAUTH_ACCOUNT_MISSING",
    ].some((prefix) => message.includes(prefix));
  }, []);

  const isTemporaryCodexSwitchError = React.useCallback((message: string) => {
    return message.includes("CODEX_OAUTH_TEMPORARY_FAILURE");
  }, []);

  const {
    accounts,
    defaultAccountId,
    currentAccountId,
    currentAccountLogin,
    hasAnyAccount,
    pollingState,
    deviceCode,
    error,
    lastCompletedAccount,
    isPolling,
    isAddingAccount,
    isRemovingAccount,
    isSettingDefaultAccount,
    isImportingCurrent,
    isSwitchingCurrent,
    isRotatingCodex,
    addAccount,
    importCurrent,
    rotateCodexAccount,
    switchCurrentAccount,
    removeAccount,
    setDefaultAccount,
    cancelAuth,
    logout,
  } = useCodexOauth();

  React.useEffect(() => {
    if (!reauthTarget || !lastCompletedAccount) {
      return;
    }

    if (lastCompletedAccount.id === reauthTarget.id) {
      toast.success(
        t("codexOauth.reauthSuccess", {
          defaultValue: `已更新 ${reauthTarget.login} 的登录态，现在可以直接再试切换`,
          account: reauthTarget.login,
        }),
      );
    } else {
      toast.error(
        t("codexOauth.reauthMismatch", {
          defaultValue: `本次登录的是 ${lastCompletedAccount.login}，不是要修复的 ${reauthTarget.login}，原账号登录态未更新`,
          actual: lastCompletedAccount.login,
          target: reauthTarget.login,
        }),
      );
    }

    setReauthTarget(null);
  }, [lastCompletedAccount, reauthTarget, t]);

  const copyUserCode = async () => {
    if (deviceCode?.user_code) {
      await copyText(deviceCode.user_code);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  const handleAccountSelect = (value: string) => {
    onAccountSelect?.(value === "none" ? null : value);
  };

  const handleRemoveAccount = (accountId: string, e: React.MouseEvent) => {
    e.stopPropagation();
    e.preventDefault();
    removeAccount(accountId);
    if (selectedAccountId === accountId) {
      onAccountSelect?.(null);
    }
  };

  const handleAddAnotherAccount = async () => {
    setReauthTarget(null);
    await addAccount();
  };

  const handleCancelAuth = () => {
    setReauthTarget(null);
    cancelAuth();
  };

  const handleReauthAccount = async (accountId: string, accountLogin: string) => {
    setReauthTarget({ id: accountId, login: accountLogin });
    toast.info(
      t("codexOauth.reauthStartHint", {
        defaultValue: `正在为 ${accountLogin} 打开重新登录流程。请在浏览器里直接完成这个账号的授权，不要先退出当前其他账号。`,
        account: accountLogin,
      }),
    );

    try {
      await addAccount();
    } catch (e) {
      setReauthTarget(null);
      const message = e instanceof Error ? e.message : String(e);
      toast.error(message);
    }
  };

  const handleSwitchCurrentAccount = async () => {
    const targetAccountId = selectedAccountId ?? null;

    try {
      const account = await switchCurrentAccount(targetAccountId);
      onAccountSelect?.(targetAccountId);
      toast.success(
        targetAccountId
          ? t("codexOauth.switchSuccess", {
              defaultValue: `已切换到 ${account.login}，重启 Codex 后生效`,
              account: account.login,
            })
          : t("codexOauth.switchToDefaultSuccess", {
              defaultValue: `已切回默认账号 ${account.login}，重启 Codex 后生效`,
              account: account.login,
            }),
      );
    } catch (e) {
      const initialMessage = e instanceof Error ? e.message : String(e);
      if (isTemporaryCodexSwitchError(initialMessage)) {
        toast.error(stripCodexSwitchErrorPrefix(initialMessage));
        return;
      }

      if (!isRecoverableCodexSwitchError(initialMessage)) {
        toast.error(stripCodexSwitchErrorPrefix(initialMessage));
        return;
      }

      try {
        toast.info(
          t("codexOauth.switchRecoveryImporting", {
            defaultValue: "正在尝试同步当前 Codex 登录态...",
          }),
        );
        await importCurrent();

        const account = await switchCurrentAccount(targetAccountId);
        onAccountSelect?.(targetAccountId);
        toast.success(
          targetAccountId
            ? t("codexOauth.switchRecovered", {
                defaultValue: `已恢复并切换到 ${account.login}`,
                account: account.login,
              })
            : t("codexOauth.switchToDefaultRecovered", {
                defaultValue: `已恢复并切回默认账号 ${account.login}`,
                account: account.login,
              }),
        );
        return;
      } catch (recoveryError) {
        const recoveryMessage =
          recoveryError instanceof Error
            ? recoveryError.message
            : String(recoveryError);

        if (isTemporaryCodexSwitchError(recoveryMessage)) {
          toast.error(stripCodexSwitchErrorPrefix(recoveryMessage));
          return;
        }
      }

      try {
        await addAccount();
        toast.info(
          t("codexOauth.switchRecoveryReauthStarted", {
            defaultValue:
              "当前账号无法静默恢复，已自动打开 ChatGPT 重新登录流程。",
          }),
        );
      } catch (loginError) {
        const loginMessage =
          loginError instanceof Error ? loginError.message : String(loginError);
        toast.error(stripCodexSwitchErrorPrefix(loginMessage));
      }
    }
  };

  const handleRotateCodexAccount = async () => {
    try {
      const result = await rotateCodexAccount(false);
      if (result.switched) {
        toast.success(
          t("codexOauth.rotateSuccess", {
            defaultValue: `已自动切换到 ${result.to_account_login ?? result.to_account_id}`,
            account: result.to_account_login ?? result.to_account_id ?? "",
          }),
        );
        if (result.to_account_id) {
          onAccountSelect?.(result.to_account_id);
        }
        return;
      }

      toast.info(
        result.reason ||
          t("codexOauth.rotateNoop", {
            defaultValue: "当前账号仍可用，无需切换",
          }),
      );
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      toast.error(message);
    }
  };

  const hasRuntimeAccountMismatch =
    !!currentAccountId &&
    !!selectedAccountId &&
    currentAccountId !== selectedAccountId;
  const isRuntimeAccountTracked =
    !currentAccountId ||
    accounts.some((account) => account.id === currentAccountId);

  return (
    <div className={`space-y-4 ${className || ""}`}>
      {/* 认证状态标题 */}
      <div className="flex items-center justify-between">
        <Label>{t("codexOauth.authStatus", "认证状态")}</Label>
        <Badge
          variant={hasAnyAccount ? "default" : "secondary"}
          className={hasAnyAccount ? "bg-green-500 hover:bg-green-600" : ""}
        >
          {hasAnyAccount
            ? t("codexOauth.accountCount", {
                count: accounts.length,
                defaultValue: `${accounts.length} 个账号`,
              })
            : t("codexOauth.notAuthenticated", "未认证")}
        </Badge>
      </div>

      {/* 账号选择器 */}
      {hasAnyAccount && onAccountSelect && (
        <div className="space-y-2">
          <Label className="text-sm text-muted-foreground">
            {t("codexOauth.selectAccount", "选择账号")}
          </Label>
          <Select
            value={selectedAccountId || "none"}
            onValueChange={handleAccountSelect}
          >
            <SelectTrigger>
              <SelectValue
                placeholder={t(
                  "codexOauth.selectAccountPlaceholder",
                  "选择一个 ChatGPT 账号",
                )}
              />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="none">
                <span className="text-muted-foreground">
                  {t("codexOauth.useDefaultAccount", "使用默认账号")}
                </span>
              </SelectItem>
              {accounts.map((account) => (
                <SelectItem key={account.id} value={account.id}>
                  <div className="flex items-center gap-2">
                    <User className="h-4 w-4 text-muted-foreground" />
                    <span>{account.login}</span>
                  </div>
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {enableDirectSwitch && (
            <div className="space-y-2 rounded-md border bg-muted/20 p-3">
              <div className="flex flex-wrap items-center gap-2 text-xs">
                <span className="text-muted-foreground">
                  {t("codexOauth.currentDesktopAccount", {
                    defaultValue: "当前 Codex 运行账号",
                  })}
                </span>
                {currentAccountId ? (
                  <Badge variant="outline" className="text-xs">
                    {currentAccountLogin ?? currentAccountId}
                  </Badge>
                ) : (
                  <Badge variant="secondary" className="text-xs">
                    {t("codexOauth.currentDesktopAccountUnknown", {
                      defaultValue: "未识别",
                    })}
                  </Badge>
                )}
              </div>
              {!isRuntimeAccountTracked && (
                <p className="text-xs text-amber-600 dark:text-amber-400">
                  {t("codexOauth.currentDesktopAccountNotTracked", {
                    defaultValue:
                      "当前桌面登录账号不在账号池里，建议先导入当前登录后再切换。",
                  })}
                </p>
              )}
              {hasRuntimeAccountMismatch && (
                <p className="text-xs text-muted-foreground">
                  {t("codexOauth.switchTargetHint", {
                    defaultValue:
                      "当前运行账号和已选账号不同，点击下方按钮后会切到选中账号。",
                  })}
                </p>
              )}
              <p className="text-xs text-muted-foreground">
                {t(
                  "codexOauth.directSwitchHint",
                  "这里会保留当前 Codex 的会话与历史，只重写 ~/.codex/auth.json，关闭并重新启动 Codex 来切换账号。",
                )}
              </p>
              <Button
                type="button"
                variant="secondary"
                className="w-full"
                onClick={() => void handleSwitchCurrentAccount()}
                disabled={isSwitchingCurrent || isRotatingCodex}
              >
                {isSwitchingCurrent ? (
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                ) : (
                  <RefreshCw className="mr-2 h-4 w-4" />
                )}
                {selectedAccountId
                  ? t("codexOauth.switchToSelected", "立即切到选中账号")
                  : t("codexOauth.switchToDefault", "立即切到默认账号")}
              </Button>
              <Button
                type="button"
                variant="outline"
                className="w-full"
                onClick={() => void handleRotateCodexAccount()}
                disabled={isRotatingCodex || isSwitchingCurrent}
              >
                {isRotatingCodex ? (
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                ) : (
                  <RefreshCw className="mr-2 h-4 w-4" />
                )}
                {t(
                  "codexOauth.rotateNextAvailable",
                  "检查额度并切下一个可用账号",
                )}
              </Button>
            </div>
          )}
        </div>
      )}

      {/* 已登录账号列表 */}
      {hasAnyAccount && (
        <div className="space-y-2">
          <Label className="text-sm text-muted-foreground">
            {t("codexOauth.loggedInAccounts", "已登录账号")}
          </Label>
          <div className="space-y-1">
            {accounts.map((account) => {
              return (
                <div
                  key={account.id}
                  className="flex items-center justify-between p-2 rounded-md border bg-muted/30"
                >
                  <div className="flex items-start gap-2">
                    <User className="h-5 w-5 text-muted-foreground" />
                    <div className="space-y-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="text-sm font-medium">
                          {account.login}
                        </span>
                        {defaultAccountId === account.id && (
                          <Badge variant="secondary" className="text-xs">
                            {t("codexOauth.defaultAccount", "默认")}
                          </Badge>
                        )}
                        {currentAccountId === account.id && (
                          <Badge variant="default" className="text-xs">
                            {t(
                              "codexOauth.currentDesktopAccountBadge",
                              "运行中",
                            )}
                          </Badge>
                        )}
                        {selectedAccountId === account.id && (
                          <Badge variant="outline" className="text-xs">
                            {t("codexOauth.selected", "已选中")}
                          </Badge>
                        )}
                      </div>
                    </div>
                  </div>
                  <div className="flex items-center gap-1">
                    {defaultAccountId !== account.id && (
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        className="h-7 px-2 text-xs text-muted-foreground"
                        onClick={() => setDefaultAccount(account.id)}
                        disabled={isSettingDefaultAccount}
                      >
                        {t("codexOauth.setAsDefault", "设为默认")}
                      </Button>
                    )}
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="h-7 px-2 text-xs text-muted-foreground"
                      onClick={() =>
                        void handleReauthAccount(account.id, account.login)
                      }
                      disabled={isAddingAccount || isImportingCurrent}
                    >
                      {reauthTarget?.id === account.id && isPolling ? (
                        <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <RefreshCw className="mr-1 h-3.5 w-3.5" />
                      )}
                      {t("codexOauth.reauthAccount", {
                        defaultValue: "重新登录",
                      })}
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon"
                      className="h-7 w-7 text-muted-foreground hover:text-red-500"
                      onClick={(e) => handleRemoveAccount(account.id, e)}
                      disabled={isRemovingAccount}
                      title={t("codexOauth.removeAccount", "移除账号")}
                    >
                      <X className="h-4 w-4" />
                    </Button>
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      )}

      {/* 未认证 - 登录按钮 */}
      {!hasAnyAccount && pollingState === "idle" && (
        <div className="space-y-2">
          <Button
            type="button"
            onClick={() => void handleAddAnotherAccount()}
            className="w-full"
            variant="outline"
            disabled={isImportingCurrent}
          >
            <Sparkles className="mr-2 h-4 w-4" />
            {t("codexOauth.loginWithChatGPT", "使用 ChatGPT 登录")}
          </Button>
          <Button
            type="button"
            onClick={importCurrent}
            className="w-full"
            variant="secondary"
            disabled={isImportingCurrent || isAddingAccount}
          >
            {isImportingCurrent ? (
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            ) : (
              <ArrowDownToLine className="mr-2 h-4 w-4" />
            )}
            {t("codexOauth.importCurrent", "导入当前 Codex 登录")}
          </Button>
        </div>
      )}

      {/* 已有账号 - 添加更多按钮 */}
      {hasAnyAccount && pollingState === "idle" && (
        <div className="space-y-2">
          <Button
            type="button"
            onClick={() => void handleAddAnotherAccount()}
            className="w-full"
            variant="outline"
            disabled={isAddingAccount || isImportingCurrent}
          >
            <Plus className="mr-2 h-4 w-4" />
            {t("codexOauth.addAnotherAccount", "添加其他账号")}
          </Button>
          <Button
            type="button"
            onClick={importCurrent}
            className="w-full"
            variant="secondary"
            disabled={isImportingCurrent || isAddingAccount}
          >
            {isImportingCurrent ? (
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            ) : (
              <ArrowDownToLine className="mr-2 h-4 w-4" />
            )}
            {t("codexOauth.importCurrent", "导入当前 Codex 登录")}
          </Button>
        </div>
      )}

      {/* 轮询中状态 */}
      {isPolling && deviceCode && (
        <div className="space-y-3 p-4 rounded-lg border border-border bg-muted/50">
          <div className="flex items-center justify-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            {t("codexOauth.waitingForAuth", "等待授权中...")}
          </div>

          {reauthTarget && (
            <div className="rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs text-amber-700 dark:border-amber-800 dark:bg-amber-950/30 dark:text-amber-300">
              {t("codexOauth.reauthPollingHint", {
                defaultValue:
                  "这次是在修复 {{account}} 的登录态。请在浏览器中直接完成这个账号的授权，成功后会覆盖原登录态。",
                account: reauthTarget.login,
              })}
            </div>
          )}

          <div className="text-center">
            <p className="text-xs text-muted-foreground mb-1">
              {t("codexOauth.enterCode", "在浏览器中输入以下代码：")}
            </p>
            <div className="flex items-center justify-center gap-2">
              <code className="text-2xl font-mono font-bold tracking-wider bg-background px-4 py-2 rounded border">
                {deviceCode.user_code}
              </code>
              <Button
                type="button"
                size="icon"
                variant="ghost"
                onClick={copyUserCode}
                title={t("codexOauth.copyCode", "复制代码")}
              >
                {copied ? (
                  <Check className="h-4 w-4 text-green-500" />
                ) : (
                  <Copy className="h-4 w-4" />
                )}
              </Button>
            </div>
          </div>

          <div className="text-center">
            <a
              href={deviceCode.verification_uri}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1 text-sm text-blue-500 hover:underline"
            >
              {deviceCode.verification_uri}
              <ExternalLink className="h-3 w-3" />
            </a>
          </div>

          <div className="text-center">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={handleCancelAuth}
            >
              {t("common.cancel", "取消")}
            </Button>
          </div>
        </div>
      )}

      {/* 错误状态 */}
      {error && (
        <div className="space-y-2">
          <p className="text-sm text-red-500">{error}</p>
          {pollingState === "error" && (
            <div className="flex gap-2">
              <Button
                type="button"
                onClick={() => void handleAddAnotherAccount()}
                variant="outline"
                size="sm"
              >
                {reauthTarget
                  ? t("codexOauth.retryReauth", {
                      defaultValue: "重试重新登录",
                    })
                  : t("codexOauth.retry", "重试")}
              </Button>
              <Button
                type="button"
                onClick={handleCancelAuth}
                variant="ghost"
                size="sm"
              >
                {t("common.cancel", "取消")}
              </Button>
            </div>
          )}
        </div>
      )}

      {/* 注销所有账号 */}
      {hasAnyAccount && accounts.length > 1 && (
        <Button
          type="button"
          variant="outline"
          onClick={logout}
          className="w-full text-red-500 hover:text-red-600 hover:bg-red-50 dark:hover:bg-red-950"
        >
          <LogOut className="mr-2 h-4 w-4" />
          {t("codexOauth.logoutAll", "注销所有账号")}
        </Button>
      )}
    </div>
  );
};

export default CodexOAuthSection;
