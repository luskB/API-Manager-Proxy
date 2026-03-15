import { useEffect, useMemo, useRef, useState } from "react";
import {
  RefreshCw,
  ScanSearch,
  CheckCircle2,
  KeyRound,
  ChartNoAxesCombined,
  Plus,
  Trash2,
  X,
  Loader2,
} from "lucide-react";
import { request } from "../utils/request";
import type { SiteAccount } from "../types/backup";
import { useLocale } from "../hooks/useLocale";

const QUOTA_FACTOR = 500000;

type JsonRecord = Record<string, unknown>;

interface HubDetectionResult {
  account_id: string;
  site_name: string;
  site_url: string;
  site_type: string;
  status: string;
  error?: string;
  balance?: number;
  today_usage?: number;
  today_prompt_tokens?: number;
  today_completion_tokens?: number;
  today_requests_count?: number;
  models: string[];
  has_checkin: boolean;
  can_check_in?: boolean;
}

interface HubCheckinResult {
  account_id: string;
  site_name: string;
  success: boolean;
  message: string;
  reward?: number;
  site_type?: string;
}

interface HubCheckinResponse {
  checkin: HubCheckinResult;
  detection?: HubDetectionResult;
}

interface HubBalanceRefreshResponse {
  accounts: SiteAccount[];
  refreshed: number;
  failed: number;
}

interface GroupRow {
  name: string;
  desc: string;
  ratio: number;
}

interface PricingRow {
  model: string;
  input?: number;
  output?: number;
  quotaType?: number;
  groups: string[];
}

interface CreateTokenForm {
  name: string;
  group: string;
  unlimitedQuota: boolean;
  quota: string;
  expiredTime: string;
}

function toNumber(value: unknown): number | undefined {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return undefined;
}

function toText(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number") return String(value);
  return "";
}

function tokenId(token: JsonRecord): string {
  const id = token.id ?? token.token_id ?? token.key ?? token.name;
  return toText(id) || crypto.randomUUID();
}

function tokenKey(token: JsonRecord): string {
  const key = token.key ?? token.token;
  return toText(key);
}

function maskKey(key: string): string {
  if (!key) return "-";
  if (key.length <= 12) return key;
  return `${key.slice(0, 8)}...${key.slice(-4)}`;
}

const HUB_STORAGE = {
  autoRefreshEnabled: "hub.autoRefreshEnabled",
  autoRefreshMinutes: "hub.autoRefreshMinutes",
} as const;

function storageGet(key: string): string | null {
  try {
    if (typeof window === "undefined") return null;
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function storageSet(key: string, value: string) {
  try {
    if (typeof window === "undefined") return;
    window.localStorage.setItem(key, value);
  } catch {
    // ignore
  }
}

function parseStoredMinutes(): number {
  const raw = storageGet(HUB_STORAGE.autoRefreshMinutes);
  const parsed = Number(raw);
  if (Number.isFinite(parsed) && parsed >= 1) {
    return Math.round(parsed);
  }
  return 5;
}

function formatCurrencyFromQuota(value?: number): string {
  if (value == null) return "-";
  return `$${(value / QUOTA_FACTOR).toFixed(2)}`;
}

function parseGroups(raw: unknown): GroupRow[] {
  if (!raw || typeof raw !== "object") return [];
  const obj = raw as Record<string, unknown>;
  return Object.entries(obj)
    .map(([name, info]) => {
      const row = (info ?? {}) as Record<string, unknown>;
      return {
        name,
        desc: toText(row.desc) || name,
        ratio: toNumber(row.ratio) ?? 1,
      };
    })
    .sort((a, b) => a.name.localeCompare(b.name));
}

function parsePricing(raw: unknown): PricingRow[] {
  if (!raw || typeof raw !== "object") return [];
  const payload = raw as Record<string, unknown>;
  const data =
    payload.data && typeof payload.data === "object"
      ? (payload.data as Record<string, unknown>)
      : payload;

  return Object.entries(data)
    .map(([model, info]) => {
      const row = (info ?? {}) as Record<string, unknown>;
      const modelPrice = row.model_price as Record<string, unknown> | number | string | undefined;
      const groups = Array.isArray(row.enable_groups)
        ? row.enable_groups.map((v) => String(v))
        : Array.isArray(row.groups)
          ? row.groups.map((v) => String(v))
          : [];

      const quotaTypeRaw = row.quota_type;
      const quotaType =
        typeof quotaTypeRaw === "string"
          ? quotaTypeRaw.toLowerCase() === "times"
            ? 1
            : 0
          : toNumber(quotaTypeRaw);
      const scalarModelPrice =
        typeof modelPrice === "number" || typeof modelPrice === "string" ? toNumber(modelPrice) : undefined;
      const nestedInput =
        typeof modelPrice === "object" && modelPrice
          ? toNumber((modelPrice as Record<string, unknown>).input)
          : undefined;
      const nestedOutput =
        typeof modelPrice === "object" && modelPrice
          ? toNumber((modelPrice as Record<string, unknown>).output)
          : undefined;
      const modelRatio = toNumber(row.model_ratio);
      const completionRatio = toNumber(row.completion_ratio) ?? 1;
      const tokenInputFallback =
        nestedInput ??
        toNumber(row.input) ??
        modelRatio ??
        (quotaType === 1 ? scalarModelPrice : undefined) ??
        scalarModelPrice;
      const input =
        quotaType === 1
          ? scalarModelPrice ?? nestedInput ?? toNumber(row.input) ?? modelRatio
          : tokenInputFallback;
      const output =
        quotaType === 1
          ? scalarModelPrice ?? nestedOutput ?? toNumber(row.output) ?? input
          : nestedOutput ??
            toNumber(row.output) ??
            (tokenInputFallback != null ? tokenInputFallback * completionRatio : undefined) ??
            scalarModelPrice;

      return {
        model,
        quotaType,
        input,
        output,
        groups,
      };
    })
    .sort((a, b) => a.model.localeCompare(b.model));
}

export default function HubPage() {
  const { t, locale } = useLocale();

  const [accounts, setAccounts] = useState<SiteAccount[]>([]);
  const [detectionMap, setDetectionMap] = useState<Record<string, HubDetectionResult>>({});
  const [selectedAccountIds, setSelectedAccountIds] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshingBalances, setRefreshingBalances] = useState(false);
  const [selectedRefreshing, setSelectedRefreshing] = useState(false);
  const [busyAccountId, setBusyAccountId] = useState<string | null>(null);
  const [detectingAll, setDetectingAll] = useState(false);
  const [autoCheckinRunning, setAutoCheckinRunning] = useState(false);
  const [selectedCheckinRunning, setSelectedCheckinRunning] = useState(false);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [search, setSearch] = useState("");
  const [typeFilter, setTypeFilter] = useState("all");
  const [autoRefreshEnabled, setAutoRefreshEnabled] = useState(
    () => storageGet(HUB_STORAGE.autoRefreshEnabled) === "true",
  );
  const [autoRefreshMinutes, setAutoRefreshMinutes] = useState<number>(parseStoredMinutes);
  const refreshInFlightRef = useRef(false);

  const [tokenModalOpen, setTokenModalOpen] = useState(false);
  const [tokenAccount, setTokenAccount] = useState<SiteAccount | null>(null);
  const [tokensLoading, setTokensLoading] = useState(false);
  const [tokens, setTokens] = useState<JsonRecord[]>([]);
  const [tokenError, setTokenError] = useState("");
  const [tokenGroups, setTokenGroups] = useState<GroupRow[]>([]);
  const [tokenForm, setTokenForm] = useState<CreateTokenForm>({
    name: "",
    group: "default",
    unlimitedQuota: true,
    quota: "",
    expiredTime: "",
  });
  const [creatingToken, setCreatingToken] = useState(false);
  const [deletingTokenRef, setDeletingTokenRef] = useState("");

  const [pricingModalOpen, setPricingModalOpen] = useState(false);
  const [pricingAccount, setPricingAccount] = useState<SiteAccount | null>(null);
  const [pricingLoading, setPricingLoading] = useState(false);
  const [pricingError, setPricingError] = useState("");
  const [pricingRows, setPricingRows] = useState<PricingRow[]>([]);
  const [pricingGroups, setPricingGroups] = useState<GroupRow[]>([]);
  const text = (zh: string, en: string) => (locale === "zh" ? zh : en);

  async function loadAccounts(showSpinner = true) {
    if (showSpinner) setLoading(true);
    try {
      const list = await request<SiteAccount[]>("list_hub_accounts");
      setAccounts(list ?? []);
      setError("");
    } catch (e) {
      setError(String(e));
    } finally {
      if (showSpinner) setLoading(false);
    }
  }

  useEffect(() => {
    void loadAccounts();
  }, []);

  useEffect(() => {
    if (!message) return;
    const timer = setTimeout(() => setMessage(""), 4000);
    return () => clearTimeout(timer);
  }, [message]);

  useEffect(() => {
    const validIds = new Set(accounts.map((account) => account.id));
    setSelectedAccountIds((prev) => prev.filter((id) => validIds.has(id)));
  }, [accounts]);

  useEffect(() => {
    storageSet(HUB_STORAGE.autoRefreshEnabled, String(autoRefreshEnabled));
  }, [autoRefreshEnabled]);

  useEffect(() => {
    storageSet(HUB_STORAGE.autoRefreshMinutes, String(autoRefreshMinutes));
  }, [autoRefreshMinutes]);

  useEffect(() => {
    if (!autoRefreshEnabled) return;

    void refreshBalanceSnapshots(true);
    const intervalMs = Math.max(1, autoRefreshMinutes) * 60 * 1000;
    const timer = window.setInterval(() => {
      void refreshBalanceSnapshots(true);
    }, intervalMs);

    return () => window.clearInterval(timer);
  }, [autoRefreshEnabled, autoRefreshMinutes]);

  const typeOptions = useMemo(() => {
    return Array.from(new Set(accounts.map((a) => a.site_type))).sort((a, b) => a.localeCompare(b));
  }, [accounts]);

  const filteredAccounts = useMemo(() => {
    const query = search.trim().toLowerCase();
    return accounts
      .filter((account) => {
        if (typeFilter !== "all" && account.site_type !== typeFilter) return false;
        if (!query) return true;
        const haystack = `${account.site_name} ${account.site_url} ${account.account_info.username}`.toLowerCase();
        return haystack.includes(query);
      })
      .sort((a, b) => a.site_name.localeCompare(b.site_name));
  }, [accounts, search, typeFilter]);

  const filteredAccountIds = useMemo(
    () => filteredAccounts.map((account) => account.id),
    [filteredAccounts],
  );

  const allFilteredSelected =
    filteredAccountIds.length > 0 &&
    filteredAccountIds.every((accountId) => selectedAccountIds.includes(accountId));

  function rowStatus(account: SiteAccount): "success" | "failed" | "unknown" {
    const detection = detectionMap[account.id];
    if (detection) {
      if (detection.status === "success") return "success";
      if (detection.status === "failed") return "failed";
    }

    const health = account.health?.status;
    if (health === "normal") return "success";
    if (health === "error") return "failed";
    return "unknown";
  }

  function checkinLabel(account: SiteAccount): string {
    const detection = detectionMap[account.id];
    if (!detection) return t("hub.checkinUnknown");
    if (!detection.has_checkin) return t("hub.checkinUnsupported");
    if (detection.can_check_in === true) return t("hub.checkinReady");
    if (detection.can_check_in === false) return t("hub.checkinDone");
    return t("hub.checkinUnknown");
  }

  async function refreshBalanceSnapshots(silent = false) {
    if (refreshInFlightRef.current) return;

    refreshInFlightRef.current = true;
    setRefreshingBalances(true);
    if (!silent) {
      setError("");
    }

    try {
      const result = await request<HubBalanceRefreshResponse>("refresh_hub_balances");
      setAccounts(result.accounts ?? []);
      if (!silent) {
        setMessage(
          result.failed > 0
            ? text(
                `已刷新 ${result.refreshed} 个账户，${result.failed} 个失败`,
                `Refreshed ${result.refreshed} accounts, ${result.failed} failed`,
              )
            : text(
                `已刷新 ${result.refreshed} 个账户的余额与今日消耗`,
                `Refreshed balance and daily usage for ${result.refreshed} accounts`,
              ),
        );
      }
    } catch (e) {
      if (!silent) {
        setError(String(e));
      }
    } finally {
      refreshInFlightRef.current = false;
      setRefreshingBalances(false);
    }
  }

  function toggleSelectedAccount(accountId: string) {
    setSelectedAccountIds((prev) =>
      prev.includes(accountId)
        ? prev.filter((id) => id !== accountId)
        : [...prev, accountId],
    );
  }

  function selectAllFilteredAccounts() {
    setSelectedAccountIds((prev) => Array.from(new Set([...prev, ...filteredAccountIds])));
  }

  function clearSelectedAccounts() {
    setSelectedAccountIds([]);
  }

  async function refreshSelectedBalances() {
    if (selectedAccountIds.length === 0) {
      setMessage(text("请先选择站点", "Select sites first"));
      return;
    }

    setSelectedRefreshing(true);
    setError("");
    try {
      const result = await request<HubBalanceRefreshResponse>("refresh_selected_hub_balances", {
        account_ids: selectedAccountIds,
      });
      setAccounts(result.accounts ?? []);
      setMessage(
        result.failed > 0
          ? text(
              `已刷新 ${result.refreshed} 个选中站点，${result.failed} 个失败`,
              `Refreshed ${result.refreshed} selected sites, ${result.failed} failed`,
            )
          : text(
              `已刷新 ${result.refreshed} 个选中站点的余额与今日消耗`,
              `Refreshed balance and daily usage for ${result.refreshed} selected sites`,
            ),
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setSelectedRefreshing(false);
    }
  }

  async function checkinSelectedAccounts() {
    if (selectedAccountIds.length === 0) {
      setMessage(text("请先选择站点", "Select sites first"));
      return;
    }

    setSelectedCheckinRunning(true);
    setError("");
    try {
      const targets = accounts.filter((account) => selectedAccountIds.includes(account.id));
      let success = 0;
      let failed = 0;
      let unsupported = 0;
      const detectionUpdates: Record<string, HubDetectionResult> = {};

      for (const account of targets) {
        const detection = detectionMap[account.id];
        if (detection && !detection.has_checkin) {
          unsupported += 1;
          continue;
        }

        const response = await request<HubCheckinResponse>("hub_checkin_account", {
          account_id: account.id,
        });

        if (response.detection) {
          detectionUpdates[response.detection.account_id] = response.detection;
        }

        if (response.checkin.success) {
          success += 1;
        } else {
          failed += 1;
        }
      }

      if (Object.keys(detectionUpdates).length > 0) {
        setDetectionMap((prev) => ({ ...prev, ...detectionUpdates }));
      }

      await loadAccounts(false);
      setMessage(
        text(
          `选中签到完成：成功 ${success}，失败 ${failed}${unsupported > 0 ? `，不支持 ${unsupported}` : ""}`,
          `Selected check-in finished: ${success} succeeded, ${failed} failed${unsupported > 0 ? `, ${unsupported} unsupported` : ""}`,
        ),
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setSelectedCheckinRunning(false);
    }
  }

  async function detectOne(account: SiteAccount) {
    setBusyAccountId(account.id);
    setError("");
    try {
      const result = await request<HubDetectionResult>("detect_hub_account", {
        account_id: account.id,
        include_details: true,
      });
      setDetectionMap((prev) => ({ ...prev, [result.account_id]: result }));
      await loadAccounts(false);
      setMessage(
        result.status === "success" ? t("hub.detectSuccess") : `${t("hub.detectFailed")}: ${result.error || "-"}`,
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyAccountId(null);
    }
  }

  async function detectAll() {
    setDetectingAll(true);
    setError("");
    try {
      const results = await request<HubDetectionResult[]>("detect_all_hub_accounts", {
        include_details: true,
      });

      const nextMap: Record<string, HubDetectionResult> = {};
      let success = 0;
      for (const result of results) {
        nextMap[result.account_id] = result;
        if (result.status === "success") success += 1;
      }
      setDetectionMap((prev) => ({ ...prev, ...nextMap }));
      await loadAccounts(false);
      setMessage(t("hub.detectAllSummary").replace("{success}", String(success)).replace("{total}", String(results.length)));
    } catch (e) {
      setError(String(e));
    } finally {
      setDetectingAll(false);
    }
  }

  async function checkinOne(account: SiteAccount) {
    setBusyAccountId(account.id);
    setError("");
    try {
      const response = await request<HubCheckinResponse>("hub_checkin_account", {
        account_id: account.id,
      });

      if (response.detection) {
        setDetectionMap((prev) => ({ ...prev, [response.detection!.account_id]: response.detection! }));
      }
      await loadAccounts(false);

      const rewardText = response.checkin.reward
        ? ` (+$${(response.checkin.reward / QUOTA_FACTOR).toFixed(4)})`
        : "";
      setMessage(`${response.checkin.message}${rewardText}`);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyAccountId(null);
    }
  }

  async function autoCheckin() {
    setAutoCheckinRunning(true);
    setError("");
    try {
      const detectionResults = await request<HubDetectionResult[]>("detect_all_hub_accounts", {
        include_details: false,
      });

      const nextMap: Record<string, HubDetectionResult> = {};
      for (const result of detectionResults) {
        nextMap[result.account_id] = result;
      }
      setDetectionMap((prev) => ({ ...prev, ...nextMap }));

      const targets = detectionResults.filter((item) => item.has_checkin && item.can_check_in !== false);
      if (targets.length === 0) {
        setMessage(t("hub.autoCheckinNone"));
        return;
      }

      let success = 0;
      for (const target of targets) {
        const result = await request<HubCheckinResponse>("hub_checkin_account", {
          account_id: target.account_id,
        });
        if (result.checkin.success) success += 1;
        if (result.detection) {
          setDetectionMap((prev) => ({ ...prev, [result.detection!.account_id]: result.detection! }));
        }
      }

      await loadAccounts(false);
      setMessage(
        t("hub.autoCheckinSummary")
          .replace("{success}", String(success))
          .replace("{total}", String(targets.length)),
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setAutoCheckinRunning(false);
    }
  }

  async function openTokenModal(account: SiteAccount) {
    setTokenModalOpen(true);
    setTokenAccount(account);
    setTokenError("");
    setTokens([]);
    setTokenGroups([]);
    setTokenForm({
      name: "",
      group: "default",
      unlimitedQuota: true,
      quota: "",
      expiredTime: "",
    });

    setTokensLoading(true);
    try {
      const [tokenList, groupData] = await Promise.all([
        request<JsonRecord[]>("hub_fetch_api_tokens", { account_id: account.id }),
        request<unknown>("hub_fetch_user_groups", { account_id: account.id }),
      ]);

      const groups = parseGroups(groupData);
      setTokens(tokenList ?? []);
      setTokenGroups(groups);
      if (groups.length > 0) {
        setTokenForm((prev) => ({ ...prev, group: groups[0].name }));
      }
    } catch (e) {
      setTokenError(String(e));
    } finally {
      setTokensLoading(false);
    }
  }

  async function createToken() {
    if (!tokenAccount) return;
    if (!tokenForm.name.trim()) {
      setTokenError(t("hub.createTokenNeedName"));
      return;
    }

    setCreatingToken(true);
    setTokenError("");
    try {
      const quotaInput = Number(tokenForm.quota || "0");
      const remainQuota = tokenForm.unlimitedQuota ? QUOTA_FACTOR : Math.max(0, Math.round(quotaInput * QUOTA_FACTOR));
      const expiredTime = tokenForm.expiredTime
        ? Math.floor(new Date(tokenForm.expiredTime).getTime() / 1000)
        : -1;

      const tokenData = {
        name: tokenForm.name.trim(),
        remain_quota: remainQuota,
        expired_time: expiredTime,
        unlimited_quota: tokenForm.unlimitedQuota,
        model_limits_enabled: false,
        model_limits: "",
        allow_ips: "",
        group: tokenForm.group || "default",
      };

      const updatedTokens = await request<JsonRecord[]>("hub_create_api_token", {
        account_id: tokenAccount.id,
        token_data: tokenData,
      });

      setTokens(updatedTokens ?? []);
      setTokenForm((prev) => ({ ...prev, name: "", quota: "", expiredTime: "" }));
      setMessage(t("hub.tokenCreateSuccess"));
    } catch (e) {
      setTokenError(String(e));
    } finally {
      setCreatingToken(false);
    }
  }

  async function deleteToken(token: JsonRecord) {
    if (!tokenAccount) return;

    const idValue = token.id ?? token.token_id;
    const keyValue = token.key ?? token.token;
    const identifier: Record<string, unknown> = {};
    if (idValue != null) identifier.id = idValue;
    if (keyValue != null) identifier.key = keyValue;
    if (Object.keys(identifier).length === 0) {
      setTokenError(t("hub.tokenDeleteNeedId"));
      return;
    }

    const deletingRef = toText(idValue ?? keyValue);
    setDeletingTokenRef(deletingRef);
    setTokenError("");
    try {
      const updatedTokens = await request<JsonRecord[]>("hub_delete_api_token", {
        account_id: tokenAccount.id,
        token_identifier: identifier,
      });
      setTokens(updatedTokens ?? []);
      setMessage(t("hub.tokenDeleteSuccess"));
    } catch (e) {
      setTokenError(String(e));
    } finally {
      setDeletingTokenRef("");
    }
  }

  async function openPricingModal(account: SiteAccount) {
    setPricingModalOpen(true);
    setPricingAccount(account);
    setPricingRows([]);
    setPricingGroups([]);
    setPricingError("");
    setPricingLoading(true);
    try {
      const [pricingRaw, groupsRaw] = await Promise.all([
        request<unknown>("hub_fetch_model_pricing", { account_id: account.id }),
        request<unknown>("hub_fetch_user_groups", { account_id: account.id }),
      ]);
      setPricingRows(parsePricing(pricingRaw));
      setPricingGroups(parseGroups(groupsRaw));
    } catch (e) {
      setPricingError(String(e));
    } finally {
      setPricingLoading(false);
    }
  }

  return (
    <div className="space-y-6">
      <div className="flex justify-between items-center flex-wrap gap-3">
        <div>
          <h1 className="text-2xl font-bold">{t("hub.title")}</h1>
          <p className="text-base-content/60 mt-1">{t("hub.subtitle")}</p>
        </div>
        <div className="flex gap-2 flex-wrap">
          <div className="flex items-center gap-2 rounded-lg border border-base-300 px-3 py-1.5 bg-base-100">
            <label className="label cursor-pointer gap-2 py-0">
              <span className="label-text text-xs">{text("自动刷新", "Auto refresh")}</span>
              <input
                type="checkbox"
                className="toggle toggle-sm toggle-primary"
                checked={autoRefreshEnabled}
                onChange={(e) => setAutoRefreshEnabled(e.target.checked)}
              />
            </label>
            <input
              className="input input-bordered input-sm w-20"
              type="number"
              min="1"
              step="1"
              value={autoRefreshMinutes}
              onChange={(e) => setAutoRefreshMinutes(Math.max(1, Number(e.target.value) || 1))}
            />
            <span className="text-xs text-base-content/50">{text("分钟", "min")}</span>
          </div>
          <button
            className="btn btn-outline btn-sm gap-2"
            onClick={() => void refreshBalanceSnapshots(false)}
            disabled={loading || refreshingBalances || selectedRefreshing || selectedCheckinRunning}
          >
            <RefreshCw size={14} className={loading || refreshingBalances ? "animate-spin" : ""} />
            {t("common.refresh")}
          </button>
          <button
            className="btn btn-outline btn-sm gap-2"
            onClick={detectAll}
            disabled={detectingAll || autoCheckinRunning || selectedCheckinRunning || loading || refreshingBalances || selectedRefreshing}
          >
            <ScanSearch size={14} className={detectingAll ? "animate-pulse" : ""} />
            {detectingAll ? t("hub.detecting") : t("hub.detectAll")}
          </button>
          <button
            className="btn btn-success btn-sm gap-2"
            onClick={autoCheckin}
            disabled={autoCheckinRunning || selectedCheckinRunning || detectingAll || loading || refreshingBalances || selectedRefreshing}
          >
            <CheckCircle2 size={14} className={autoCheckinRunning ? "animate-pulse" : ""} />
            {autoCheckinRunning ? t("hub.running") : t("hub.autoCheckin")}
          </button>
        </div>
      </div>

      {error && (
        <div role="alert" className="alert alert-error">
          <span className="flex-1">{error}</span>
          <button className="btn btn-ghost btn-xs" onClick={() => setError("")}>
            <X size={14} />
          </button>
        </div>
      )}

      {message && (
        <div role="status" className="alert alert-success">
          <span className="flex-1">{message}</span>
          <button className="btn btn-ghost btn-xs" onClick={() => setMessage("")}>
            <X size={14} />
          </button>
        </div>
      )}

      <div className="card bg-base-100 border border-base-300">
        <div className="p-3 border-b border-base-200 flex flex-wrap gap-2 items-end">
          <div className="form-control">
            <label className="label label-text text-xs">{t("accounts.filterSite")}</label>
            <input
              className="input input-bordered input-sm w-72"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder={t("hub.searchPlaceholder")}
            />
          </div>
          <div className="form-control">
            <label className="label label-text text-xs">{t("accounts.filterType")}</label>
            <select className="select select-bordered select-sm w-44" value={typeFilter} onChange={(e) => setTypeFilter(e.target.value)}>
              <option value="all">{t("hub.filterAll")}</option>
              {typeOptions.map((opt) => (
                <option key={opt} value={opt}>
                  {opt}
                </option>
              ))}
            </select>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <div className="rounded-lg border border-base-300 bg-base-100 px-3 py-2 text-xs text-base-content/65">
              {text("已选站点", "Selected sites")}: {selectedAccountIds.length}
            </div>
            <button
              className="btn btn-ghost btn-sm"
              onClick={selectAllFilteredAccounts}
              disabled={filteredAccountIds.length === 0 || allFilteredSelected}
            >
              {text("全选当前结果", "Select all visible")}
            </button>
            <button
              className="btn btn-ghost btn-sm"
              onClick={clearSelectedAccounts}
              disabled={selectedAccountIds.length === 0}
            >
              {text("清除选中", "Clear selected")}
            </button>
            <button
              className="btn btn-outline btn-sm gap-2"
              onClick={() => void refreshSelectedBalances()}
              disabled={
                selectedAccountIds.length === 0 ||
                selectedRefreshing ||
                refreshingBalances ||
                autoCheckinRunning ||
                selectedCheckinRunning ||
                detectingAll
              }
            >
              <RefreshCw size={14} className={selectedRefreshing ? "animate-spin" : ""} />
              {text("刷新选中", "Refresh selected")}
            </button>
            <button
              className="btn btn-success btn-sm gap-2"
              onClick={() => void checkinSelectedAccounts()}
              disabled={
                selectedAccountIds.length === 0 ||
                selectedCheckinRunning ||
                autoCheckinRunning ||
                detectingAll ||
                loading ||
                refreshingBalances ||
                selectedRefreshing
              }
            >
              <CheckCircle2 size={14} className={selectedCheckinRunning ? "animate-pulse" : ""} />
              {selectedCheckinRunning ? t("hub.running") : text("签到选中", "Check in selected")}
            </button>
          </div>
          <div className="ml-auto text-sm text-base-content/60">
            {t("common.total")}: {filteredAccounts.length}
          </div>
        </div>

        {loading ? (
          <div className="p-10 text-center text-base-content/50">{t("hub.loading")}</div>
        ) : filteredAccounts.length === 0 ? (
          <div className="p-10 text-center text-base-content/50">
            <div>{t("hub.noAccounts")}</div>
            <div className="text-xs mt-1">{t("hub.noAccountsHint")}</div>
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="table table-sm">
              <thead>
                <tr>
                  <th className="w-12">
                    <input
                      type="checkbox"
                      className="checkbox checkbox-sm"
                      checked={allFilteredSelected}
                      onChange={(event) => {
                        if (event.target.checked) {
                          selectAllFilteredAccounts();
                        } else {
                          setSelectedAccountIds((prev) =>
                            prev.filter((id) => !filteredAccountIds.includes(id)),
                          );
                        }
                      }}
                    />
                  </th>
                  <th>{t("hub.site")}</th>
                  <th>{t("hub.type")}</th>
                  <th>{t("hub.username")}</th>
                  <th>{t("hub.balance")}</th>
                  <th>{t("hub.todayUsage")}</th>
                  <th>{t("hub.models")}</th>
                  <th>{t("hub.checkin")}</th>
                  <th>{t("hub.status")}</th>
                  <th>{t("hub.actions")}</th>
                </tr>
              </thead>
              <tbody>
                {filteredAccounts.map((account) => {
                  const detection = detectionMap[account.id];
                  const status = rowStatus(account);
                  const isBusy = busyAccountId === account.id;
                  const isSelected = selectedAccountIds.includes(account.id);
                  const balance = account.account_info.quota;
                  const todayUsage = account.account_info.today_quota_consumption;

                  return (
                    <tr key={account.id}>
                      <td>
                        <input
                          type="checkbox"
                          className="checkbox checkbox-sm"
                          checked={isSelected}
                          onChange={() => toggleSelectedAccount(account.id)}
                        />
                      </td>
                      <td>
                        <div className="font-medium">{account.site_name}</div>
                        <div className="text-xs text-base-content/50">{account.site_url}</div>
                      </td>
                      <td>
                        <span className="badge badge-outline badge-sm">{account.site_type}</span>
                      </td>
                      <td className="font-mono text-xs">{account.account_info.username || "-"}</td>
                      <td className="font-mono">{formatCurrencyFromQuota(balance)}</td>
                      <td className="font-mono">{formatCurrencyFromQuota(todayUsage)}</td>
                      <td>{detection?.models.length ?? "-"}</td>
                      <td>
                        <span className="text-xs">{checkinLabel(account)}</span>
                      </td>
                      <td>
                        <span
                          className={`badge badge-sm ${
                            status === "success"
                              ? "badge-success"
                              : status === "failed"
                                ? "badge-error"
                                : "badge-ghost"
                          }`}
                        >
                          {status === "success"
                            ? t("hub.statusSuccess")
                            : status === "failed"
                              ? t("hub.statusFailed")
                              : t("hub.statusUnknown")}
                        </span>
                      </td>
                      <td>
                        <div className="flex gap-1 flex-wrap">
                          <button className="btn btn-ghost btn-xs" onClick={() => detectOne(account)} disabled={isBusy || detectingAll || autoCheckinRunning || selectedCheckinRunning || selectedRefreshing}>
                            {isBusy ? <Loader2 size={12} className="animate-spin" /> : <ScanSearch size={12} />} {t("hub.actionDetect")}
                          </button>
                          <button className="btn btn-ghost btn-xs" onClick={() => checkinOne(account)} disabled={isBusy || detectingAll || autoCheckinRunning || selectedCheckinRunning || selectedRefreshing}>
                            <CheckCircle2 size={12} /> {t("hub.actionCheckin")}
                          </button>
                          <button className="btn btn-ghost btn-xs" onClick={() => openTokenModal(account)} disabled={isBusy || detectingAll || autoCheckinRunning || selectedCheckinRunning || selectedRefreshing}>
                            <KeyRound size={12} /> {t("hub.actionTokens")}
                          </button>
                          <button className="btn btn-ghost btn-xs" onClick={() => openPricingModal(account)} disabled={isBusy || detectingAll || autoCheckinRunning || selectedCheckinRunning || selectedRefreshing}>
                            <ChartNoAxesCombined size={12} /> {t("hub.actionPricing")}
                          </button>
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {tokenModalOpen && tokenAccount && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50 backdrop-blur-sm" onClick={() => setTokenModalOpen(false)}>
          <div className="bg-base-100 rounded-2xl shadow-2xl border border-base-300 w-full max-w-5xl max-h-[90vh] overflow-hidden" onClick={(e) => e.stopPropagation()}>
            <div className="px-6 py-4 border-b border-base-200 flex items-center justify-between bg-base-200/30">
              <h3 className="font-bold">
                {t("hub.tokensTitle")}: {tokenAccount.site_name}
              </h3>
              <button className="btn btn-ghost btn-sm" onClick={() => setTokenModalOpen(false)}>
                <X size={18} />
              </button>
            </div>

            <div className="p-6 space-y-4 overflow-auto max-h-[calc(90vh-72px)]">
              {tokenError && <div className="alert alert-error">{tokenError}</div>}

              <div className="card bg-base-200/40 border border-base-300">
                <div className="card-body gap-3">
                  <h4 className="card-title text-sm">{t("hub.tokenCreate")}</h4>
                  <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-5 gap-2">
                    <input
                      className="input input-bordered input-sm"
                      placeholder={t("hub.tokenFormName")}
                      value={tokenForm.name}
                      onChange={(e) => setTokenForm((prev) => ({ ...prev, name: e.target.value }))}
                    />
                    <select
                      className="select select-bordered select-sm"
                      value={tokenForm.group}
                      onChange={(e) => setTokenForm((prev) => ({ ...prev, group: e.target.value }))}
                    >
                      {tokenGroups.length === 0 ? <option value="default">default</option> : null}
                      {tokenGroups.map((group) => (
                        <option key={group.name} value={group.name}>
                          {group.name}
                        </option>
                      ))}
                    </select>
                    <label className="label cursor-pointer justify-start gap-2">
                      <input
                        type="checkbox"
                        className="toggle toggle-sm toggle-primary"
                        checked={tokenForm.unlimitedQuota}
                        onChange={(e) => setTokenForm((prev) => ({ ...prev, unlimitedQuota: e.target.checked }))}
                      />
                      <span className="label-text text-xs">{t("hub.tokenFormUnlimited")}</span>
                    </label>
                    <input
                      className="input input-bordered input-sm"
                      type="number"
                      step="0.01"
                      min="0"
                      disabled={tokenForm.unlimitedQuota}
                      placeholder={t("hub.tokenFormQuota")}
                      value={tokenForm.quota}
                      onChange={(e) => setTokenForm((prev) => ({ ...prev, quota: e.target.value }))}
                    />
                    <input
                      className="input input-bordered input-sm"
                      type="datetime-local"
                      value={tokenForm.expiredTime}
                      onChange={(e) => setTokenForm((prev) => ({ ...prev, expiredTime: e.target.value }))}
                    />
                  </div>
                  <div className="flex justify-end">
                    <button className="btn btn-primary btn-sm gap-2" onClick={createToken} disabled={creatingToken || tokensLoading}>
                      {creatingToken ? <Loader2 size={14} className="animate-spin" /> : <Plus size={14} />} {t("hub.tokenCreateSubmit")}
                    </button>
                  </div>
                </div>
              </div>

              <div className="card bg-base-100 border border-base-300">
                <div className="card-body p-0">
                  {tokensLoading ? (
                    <div className="p-6 text-center text-base-content/50">{t("hub.tokenLoading")}</div>
                  ) : tokens.length === 0 ? (
                    <div className="p-6 text-center text-base-content/50">{t("hub.tokenNoData")}</div>
                  ) : (
                    <div className="overflow-x-auto">
                      <table className="table table-sm">
                        <thead>
                          <tr>
                            <th>{t("hub.tokenName")}</th>
                            <th>{t("hub.tokenKey")}</th>
                            <th>{t("hub.tokenGroup")}</th>
                            <th>{t("hub.tokenQuota")}</th>
                            <th>{t("hub.tokenStatus")}</th>
                            <th>{t("hub.tokenActions")}</th>
                          </tr>
                        </thead>
                        <tbody>
                          {tokens.map((token) => {
                            const id = tokenId(token);
                            const key = tokenKey(token);
                            const name = toText(token.name) || "-";
                            const group = toText(token.group) || "default";
                            const status = toNumber(token.status) ?? 1;
                            const quota = toNumber(token.remain_quota);
                            const unlimited = token.unlimited_quota === true;
                            const deleting = deletingTokenRef === id;

                            return (
                              <tr key={id}>
                                <td>{name}</td>
                                <td className="font-mono text-xs">{maskKey(key)}</td>
                                <td>{group}</td>
                                <td className="font-mono">
                                  {unlimited ? t("hub.tokenUnlimited") : formatCurrencyFromQuota(quota)}
                                </td>
                                <td>
                                  <span className={`badge badge-sm ${status === 1 ? "badge-success" : "badge-error"}`}>
                                    {status === 1 ? t("common.enabled") : t("common.disabled")}
                                  </span>
                                </td>
                                <td>
                                  <button className="btn btn-ghost btn-xs text-error" onClick={() => deleteToken(token)} disabled={deleting || creatingToken}>
                                    {deleting ? <Loader2 size={12} className="animate-spin" /> : <Trash2 size={12} />} {t("hub.tokenDelete")}
                                  </button>
                                </td>
                              </tr>
                            );
                          })}
                        </tbody>
                      </table>
                    </div>
                  )}
                </div>
              </div>
            </div>
          </div>
        </div>
      )}

      {pricingModalOpen && pricingAccount && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/50 backdrop-blur-sm" onClick={() => setPricingModalOpen(false)}>
          <div className="bg-base-100 rounded-2xl shadow-2xl border border-base-300 w-full max-w-6xl max-h-[90vh] overflow-hidden" onClick={(e) => e.stopPropagation()}>
            <div className="px-6 py-4 border-b border-base-200 flex items-center justify-between bg-base-200/30">
              <h3 className="font-bold">
                {t("hub.pricingTitle")}: {pricingAccount.site_name}
              </h3>
              <button className="btn btn-ghost btn-sm" onClick={() => setPricingModalOpen(false)}>
                <X size={18} />
              </button>
            </div>

            <div className="p-6 space-y-4 overflow-auto max-h-[calc(90vh-72px)]">
              {pricingError && <div className="alert alert-error">{pricingError}</div>}

              {pricingLoading ? (
                <div className="text-center text-base-content/50 py-8">{t("hub.pricingLoading")}</div>
              ) : (
                <>
                  <div className="card bg-base-100 border border-base-300">
                    <div className="card-body p-0">
                      {pricingRows.length === 0 ? (
                        <div className="p-6 text-center text-base-content/50">{t("hub.pricingNoData")}</div>
                      ) : (
                        <div className="overflow-x-auto">
                          <table className="table table-sm">
                            <thead>
                              <tr>
                                <th>{t("hub.pricingModel")}</th>
                                <th>{t("hub.pricingInput")}</th>
                                <th>{t("hub.pricingOutput")}</th>
                                <th>{t("hub.pricingType")}</th>
                                <th>{t("hub.pricingGroups")}</th>
                              </tr>
                            </thead>
                            <tbody>
                              {pricingRows.map((row) => (
                                <tr key={row.model}>
                                  <td className="font-mono text-xs">{row.model}</td>
                                  <td className="font-mono text-xs">{row.input ?? "-"}</td>
                                  <td className="font-mono text-xs">{row.output ?? "-"}</td>
                                  <td>{row.quotaType === 1 ? t("hub.pricingTypeTimes") : t("hub.pricingTypeToken")}</td>
                                  <td className="text-xs">{row.groups.length > 0 ? row.groups.join(", ") : "-"}</td>
                                </tr>
                              ))}
                            </tbody>
                          </table>
                        </div>
                      )}
                    </div>
                  </div>

                  <div className="card bg-base-100 border border-base-300">
                    <div className="card-body p-0">
                      <div className="p-4 border-b border-base-200 font-medium text-sm">{t("hub.groupsTitle")}</div>
                      {pricingGroups.length === 0 ? (
                        <div className="p-6 text-center text-base-content/50">{t("hub.groupsNoData")}</div>
                      ) : (
                        <div className="overflow-x-auto">
                          <table className="table table-sm">
                            <thead>
                              <tr>
                                <th>{t("hub.groupName")}</th>
                                <th>{t("hub.groupDesc")}</th>
                                <th>{t("hub.groupRatio")}</th>
                              </tr>
                            </thead>
                            <tbody>
                              {pricingGroups.map((group) => (
                                <tr key={group.name}>
                                  <td>{group.name}</td>
                                  <td>{group.desc}</td>
                                  <td className="font-mono">{group.ratio}</td>
                                </tr>
                              ))}
                            </tbody>
                          </table>
                        </div>
                      )}
                    </div>
                  </div>
                </>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
