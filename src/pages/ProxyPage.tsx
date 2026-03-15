import { useEffect, useMemo, useState } from "react";
import { request } from "../utils/request";
import type { AppConfig } from "../types/backup";
import {
  ArrowDownUp,
  Check,
  Copy,
  DollarSign,
  KeyRound,
  Pencil,
  Play,
  Plus,
  Save,
  Search,
  Square,
  Trash2,
  X,
} from "lucide-react";
import CliSyncCard from "../components/CliSyncCard";
import PageSkeleton from "../components/PageSkeleton";
import { useConfig } from "../hooks/useConfig";
import { useLocale } from "../hooks/useLocale";

interface ProxyStatus {
  running: boolean;
}

interface ProxyModelCatalogRow {
  account_id: string;
  account_selector: string;
  site_name: string;
  models: string[];
}

interface ProxyModelPriceQuote {
  billing_mode: "tokens" | "requests" | "mixed";
  source_count: number;
  from_site_pricing: boolean;
  input_per_million: number;
  output_per_million: number;
  input_per_million_max?: number | null;
  output_per_million_max?: number | null;
  request_price?: number | null;
  request_price_max?: number | null;
}

type ProxyUserKey = NonNullable<AppConfig["proxy"]["api_keys"]>[number];

interface ProxyKeyEditor {
  key: string;
  label: string;
  enabled: boolean;
  daily_limit: number;
  monthly_limit: number;
  allowed_account_ids: string[];
  allowed_models: string[];
  created_at: number;
}

type ModelPriceSort = "default" | "asc" | "desc";

interface HoverPreviewState {
  title?: string;
  content: string;
  left: number;
  top: number;
  placement: "top" | "bottom";
}

function newKeyValue(): string {
  return `sk-${crypto.randomUUID().replace(/-/g, "").slice(0, 32)}`;
}

function defaultEditor(): ProxyKeyEditor {
  return {
    key: newKeyValue(),
    label: "",
    enabled: true,
    daily_limit: 0,
    monthly_limit: 0,
    allowed_account_ids: [],
    allowed_models: [],
    created_at: Math.floor(Date.now() / 1000),
  };
}

function dedupe(values: string[]): string[] {
  return Array.from(new Set(values.filter(Boolean))).sort((a, b) => a.localeCompare(b));
}

function maskKey(value: string): string {
  if (!value) return "-";
  if (value.length <= 14) return value;
  return `${value.slice(0, 8)}...${value.slice(-4)}`;
}

function formatUsdPerMillion(value: number): string {
  if (value >= 10) return value.toFixed(2);
  if (value >= 1) return value.toFixed(3);
  if (value >= 0.1) return value.toFixed(4);
  return value.toFixed(6);
}

function formatUsdValue(value: number): string {
  if (value >= 10) return value.toFixed(2);
  if (value >= 1) return value.toFixed(3);
  if (value >= 0.1) return value.toFixed(4);
  if (value >= 0.01) return value.toFixed(5);
  return value.toFixed(6);
}

function formatRange(
  minValue?: number | null,
  maxValue?: number | null,
  formatter: (value: number) => string = formatUsdValue,
): string | null {
  if (minValue == null && maxValue == null) return null;
  const min = minValue ?? maxValue ?? 0;
  const max = maxValue ?? minValue ?? min;
  if (Math.abs(min - max) < 1e-9) {
    return formatter(min);
  }
  return `${formatter(min)}-${formatter(max)}`;
}

function modelPriceSortValue(price?: ProxyModelPriceQuote): number | null {
  if (!price) return null;

  if (price.billing_mode === "requests") {
    return price.request_price ?? price.request_price_max ?? null;
  }

  const input = price.input_per_million ?? price.input_per_million_max ?? null;
  const output =
    price.output_per_million ??
    price.output_per_million_max ??
    input;

  if (input == null && output == null) {
    return price.request_price ?? price.request_price_max ?? null;
  }

  return (input ?? 0) + (output ?? 0);
}

export default function ProxyPage() {
  const { config, setConfig, error, setError } = useConfig();
  const [status, setStatus] = useState<ProxyStatus>({ running: false });
  const [loading, setLoading] = useState(false);
  const [statusLoaded, setStatusLoaded] = useState(false);
  const [saveStatus, setSaveStatus] = useState("");
  const [catalog, setCatalog] = useState<ProxyModelCatalogRow[]>([]);
  const [catalogLoading, setCatalogLoading] = useState(false);
  const [modelPrices, setModelPrices] = useState<Record<string, ProxyModelPriceQuote>>({});
  const [editorOpen, setEditorOpen] = useState(false);
  const [editingIndex, setEditingIndex] = useState<number | null>(null);
  const [editor, setEditor] = useState<ProxyKeyEditor>(defaultEditor);
  const [editorBusy, setEditorBusy] = useState(false);
  const [modelSearchInput, setModelSearchInput] = useState("");
  const [modelSearch, setModelSearch] = useState("");
  const [priceSort, setPriceSort] = useState<ModelPriceSort>("default");
  const [hoverPreview, setHoverPreview] = useState<HoverPreviewState | null>(null);
  const { t, locale } = useLocale();

  const text = (zh: string, en: string) => (locale === "zh" ? zh : en);

  useEffect(() => {
    if (!config || statusLoaded) return;
    let cancelled = false;
    request<ProxyStatus>("get_proxy_status")
      .then((nextStatus) => {
        if (!cancelled) {
          setStatus(nextStatus);
          setStatusLoaded(true);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setStatusLoaded(true);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [config, statusLoaded]);

  useEffect(() => {
    if (!saveStatus) return;
    const timer = setTimeout(() => setSaveStatus(""), 3000);
    return () => clearTimeout(timer);
  }, [saveStatus]);

  useEffect(() => {
    if (!hoverPreview) return;

    const dismiss = () => setHoverPreview(null);
    window.addEventListener("scroll", dismiss, true);
    window.addEventListener("resize", dismiss);

    return () => {
      window.removeEventListener("scroll", dismiss, true);
      window.removeEventListener("resize", dismiss);
    };
  }, [hoverPreview]);

  useEffect(() => {
    if (!config) return;
    let cancelled = false;
    setCatalogLoading(true);
    request<ProxyModelCatalogRow[]>("get_proxy_model_catalog")
      .then((rows) => {
        if (!cancelled) {
          setCatalog(rows ?? []);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setCatalog([]);
        }
      })
      .finally(() => {
        if (!cancelled) {
          setCatalogLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [config?.proxy_accounts]);

  useEffect(() => {
    if (!editorOpen) {
      setModelPrices({});
      return;
    }

    const rows =
      editor.allowed_account_ids.length === 0
        ? catalog
        : catalog.filter((row) => editor.allowed_account_ids.includes(row.account_id));
    const models = dedupe(rows.flatMap((row) => row.models));
    if (models.length === 0) {
      setModelPrices({});
      return;
    }

    let cancelled = false;
    request<Record<string, ProxyModelPriceQuote>>("get_proxy_model_prices", {
      models,
      account_ids: editor.allowed_account_ids,
    })
      .then((prices) => {
        if (!cancelled) {
          setModelPrices(prices ?? {});
        }
      })
      .catch(() => {
        if (!cancelled) {
          setModelPrices({});
        }
      });

    return () => {
      cancelled = true;
    };
  }, [catalog, editor.allowed_account_ids, editorOpen]);

  const proxyKeys = config?.proxy.api_keys ?? [];
  const activeAccounts = config?.proxy_accounts.filter((a) => !a.disabled).length ?? 0;

  const siteOptions = useMemo(() => {
    if (!config) return [];
    const byAccount = new Map(catalog.map((row) => [row.account_id, row]));
    return config.proxy_accounts
      .filter((account) => !account.disabled)
      .map((account) => ({
        accountId: account.id,
        label: `${account.site_name}${account.account_info.username ? ` (${account.account_info.username})` : ""}`,
        selector: byAccount.get(account.id)?.account_selector ?? account.site_name,
        modelCount: byAccount.get(account.id)?.models.length ?? 0,
      }))
      .sort((a, b) => a.label.localeCompare(b.label));
  }, [catalog, config]);

  const availableModels = useMemo(() => {
    const rows =
      editor.allowed_account_ids.length === 0
        ? catalog
        : catalog.filter((row) => editor.allowed_account_ids.includes(row.account_id));
    return dedupe(rows.flatMap((row) => row.models));
  }, [catalog, editor.allowed_account_ids]);

  const visibleModels = useMemo(() => {
    const keyword = modelSearch.trim().toLowerCase();
    const filteredModels = keyword
      ? availableModels.filter((model) => model.toLowerCase().includes(keyword))
      : [...availableModels];

    if (priceSort === "default") {
      return filteredModels;
    }

    return [...filteredModels].sort((left, right) => {
      const leftValue = modelPriceSortValue(modelPrices[left]);
      const rightValue = modelPriceSortValue(modelPrices[right]);
      const leftHasPrice = leftValue != null;
      const rightHasPrice = rightValue != null;

      if (!leftHasPrice && !rightHasPrice) {
        return left.localeCompare(right);
      }
      if (!leftHasPrice) return 1;
      if (!rightHasPrice) return -1;

      if (leftValue !== rightValue) {
        return priceSort === "asc" ? leftValue! - rightValue! : rightValue! - leftValue!;
      }

      return left.localeCompare(right);
    });
  }, [availableModels, modelPrices, modelSearch, priceSort]);

  async function persistConfig(nextConfig: AppConfig) {
    await request("save_config", { config_data: nextConfig });
    setConfig(nextConfig);
    setSaveStatus(t("proxy.saved"));
  }

  async function handleStart() {
    if (!config) return;
    setLoading(true);
    setError("");
    try {
      await request("proxy_start", { config_data: config });
      setStatus({ running: true });
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  async function handleStop() {
    setLoading(true);
    setError("");
    try {
      await request("proxy_stop");
      setStatus({ running: false });
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  async function handleSaveConfig() {
    if (!config || loading) return;
    setError("");
    try {
      await persistConfig(config);
    } catch (e) {
      setError(String(e));
    }
  }

  function openCreateKey() {
    setEditingIndex(null);
    setEditor(defaultEditor());
    setModelSearchInput("");
    setModelSearch("");
    setPriceSort("default");
    setHoverPreview(null);
    setEditorOpen(true);
  }

  function openEditKey(index: number) {
    const key = proxyKeys[index];
    if (!key) return;
    setEditingIndex(index);
    setEditor({
      key: key.key,
      label: key.label,
      enabled: key.enabled,
      daily_limit: key.daily_limit,
      monthly_limit: key.monthly_limit,
      allowed_account_ids: [...(key.allowed_account_ids ?? [])],
      allowed_models: [...key.allowed_models],
      created_at: key.created_at,
    });
    setModelSearchInput("");
    setModelSearch("");
    setPriceSort("default");
    setHoverPreview(null);
    setEditorOpen(true);
  }

  function closeEditor() {
    if (editorBusy) return;
    setEditorOpen(false);
    setEditingIndex(null);
    setEditor(defaultEditor());
    setModelSearchInput("");
    setModelSearch("");
    setPriceSort("default");
    setHoverPreview(null);
  }

  function toggleSite(accountId: string) {
    setEditor((prev) => {
      const allowed_account_ids = prev.allowed_account_ids.includes(accountId)
        ? prev.allowed_account_ids.filter((value) => value !== accountId)
        : [...prev.allowed_account_ids, accountId];
      const rows =
        allowed_account_ids.length === 0
          ? catalog
          : catalog.filter((row) => allowed_account_ids.includes(row.account_id));
      const models = dedupe(rows.flatMap((row) => row.models));
      return {
        ...prev,
        allowed_account_ids,
        allowed_models: prev.allowed_models.filter((model) => models.includes(model)),
      };
    });
  }

  function toggleModel(model: string) {
    setEditor((prev) => ({
      ...prev,
      allowed_models: prev.allowed_models.includes(model)
        ? prev.allowed_models.filter((value) => value !== model)
        : [...prev.allowed_models, model],
    }));
  }

  function applyModelSearch() {
    setModelSearch(modelSearchInput.trim());
  }

  function clearModelSearch() {
    setModelSearchInput("");
    setModelSearch("");
  }

  function showHoverPreview(target: HTMLElement, content: string, title?: string) {
    const rect = target.getBoundingClientRect();
    const maxWidth = Math.min(360, window.innerWidth - 32);
    const halfWidth = maxWidth / 2;
    const placement: "top" | "bottom" = rect.top < 120 ? "bottom" : "top";
    const center = rect.left + rect.width / 2;
    const left = Math.min(
      Math.max(center, 16 + halfWidth),
      window.innerWidth - 16 - halfWidth,
    );
    const top = placement === "top" ? rect.top - 14 : rect.bottom + 14;

    setHoverPreview({
      title,
      content,
      left,
      top,
      placement,
    });
  }

  function hideHoverPreview() {
    setHoverPreview(null);
  }

  function legacyPriceTooltip(model: string): string {
    const price = modelPrices[model];
    if (!price) {
      return text("Pricing unavailable", "Pricing unavailable");
    }
    return text(
      `输入 $${formatUsdPerMillion(price.input_per_million)}/1M · 输出 $${formatUsdPerMillion(price.output_per_million)}/1M`,
      `Input $${formatUsdPerMillion(price.input_per_million)}/1M · Output $${formatUsdPerMillion(price.output_per_million)}/1M`,
    );
  }

  function priceTooltip(model: string): string {
    const price = modelPrices[model];
    if (!price) {
      return text("Pricing unavailable", "Pricing unavailable");
    }
    const requestRange = formatRange(price.request_price, price.request_price_max);
    const inputRange = formatRange(price.input_per_million, price.input_per_million_max, formatUsdPerMillion);
    const outputRange = formatRange(price.output_per_million, price.output_per_million_max, formatUsdPerMillion);
    const sourcePrefix = price.from_site_pricing
      ? price.source_count > 1
        ? text(`已匹配 ${price.source_count} 个站点`, `${price.source_count} matched sites`)
        : text("按站点真实定价", "Using site pricing")
      : text("通用参考定价", "Generic reference pricing");

    if (price.billing_mode === "requests" && requestRange) {
      return text(
        `${sourcePrefix} · $${requestRange}/次`,
        `${sourcePrefix} · $${requestRange} / request`,
      );
    }

    if (price.billing_mode === "mixed") {
      const parts = [
        requestRange ? text(`按次 $${requestRange}/次`, `Per request $${requestRange}/request`) : null,
        inputRange && outputRange
          ? text(`Token 输入 $${inputRange}/1M，输出 $${outputRange}/1M`, `Token input $${inputRange}/1M, output $${outputRange}/1M`)
          : null,
      ].filter(Boolean);
      return parts.length > 0
        ? `${sourcePrefix} · ${parts.join(" · ")}`
        : text("Pricing varies by site", "Pricing varies by site");
    }

    if (inputRange || outputRange) {
      return text(
        `${sourcePrefix} · 输入 $${inputRange ?? "-"}/1M · 输出 $${outputRange ?? inputRange ?? "-"}/1M`,
        `${sourcePrefix} · Input $${inputRange ?? "-"}/1M · Output $${outputRange ?? inputRange ?? "-"}/1M`,
      );
    }

    return legacyPriceTooltip(model);
  }

  async function saveEditor() {
    if (!config || editorBusy) return;
    setEditorBusy(true);
    setError("");
    try {
      const nextKey: ProxyUserKey = {
        key: editor.key.trim() || newKeyValue(),
        label: editor.label.trim() || `Key ${proxyKeys.length + 1}`,
        enabled: editor.enabled,
        daily_limit: Number(editor.daily_limit) || 0,
        monthly_limit: Number(editor.monthly_limit) || 0,
        allowed_account_ids: dedupe(editor.allowed_account_ids),
        allowed_models: dedupe(editor.allowed_models),
        created_at: editor.created_at || Math.floor(Date.now() / 1000),
      };
      const nextKeys = [...proxyKeys];
      if (editingIndex == null) {
        nextKeys.unshift(nextKey);
      } else {
        nextKeys[editingIndex] = nextKey;
      }
      await persistConfig({
        ...config,
        proxy: {
          ...config.proxy,
          api_keys: nextKeys,
        },
      });
      closeEditor();
    } catch (e) {
      setError(String(e));
    } finally {
      setEditorBusy(false);
    }
  }

  async function toggleKeyEnabled(index: number) {
    if (!config || !proxyKeys[index]) return;
    const nextKeys = [...proxyKeys];
    nextKeys[index] = {
      ...nextKeys[index],
      enabled: !nextKeys[index].enabled,
    };
    try {
      await persistConfig({
        ...config,
        proxy: {
          ...config.proxy,
          api_keys: nextKeys,
        },
      });
    } catch (e) {
      setError(String(e));
    }
  }

  async function deleteKey(index: number) {
    if (!config) return;
    try {
      await persistConfig({
        ...config,
        proxy: {
          ...config.proxy,
          api_keys: proxyKeys.filter((_, currentIndex) => currentIndex !== index),
        },
      });
    } catch (e) {
      setError(String(e));
    }
  }

  function copyKey(value: string) {
    navigator.clipboard.writeText(value).catch(() => {});
  }

  if (!config) {
    return <PageSkeleton />;
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between flex-wrap gap-3">
        <div>
          <h1 className="text-2xl font-bold">{t("proxy.title")}</h1>
          <p className="text-base-content/60 text-sm">{t("proxy.subtitle")}</p>
        </div>
        <div className="flex items-center gap-3">
          <span className="text-xs text-base-content/50">
            {activeAccounts} {t("proxy.activeAccounts")}
          </span>
          <span className={`badge badge-sm ${status.running ? "badge-success" : "badge-error"}`}>
            {status.running ? t("common.running") : t("common.stopped")}
          </span>
          {status.running ? (
            <button className="btn btn-error btn-sm btn-outline gap-1.5" onClick={handleStop} disabled={loading}>
              <Square size={14} />
              {t("proxy.stopProxy")}
            </button>
          ) : (
            <button
              className="btn btn-primary btn-sm gap-1.5"
              onClick={handleStart}
              disabled={loading || activeAccounts === 0}
            >
              <Play size={14} />
              {t("proxy.startProxy")}
            </button>
          )}
        </div>
      </div>

      {error && (
        <div role="alert" className="alert alert-error">
          <span>{error}</span>
        </div>
      )}

      {saveStatus && (
        <div role="status" className="alert alert-success">
          <span>{saveStatus}</span>
        </div>
      )}

      <div className="card bg-base-100 border border-base-300">
        <div className="card-body gap-3">
          <h2 className="card-title text-sm font-medium text-base-content/60">{t("proxy.configuration")}</h2>
          <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
            <label className="form-control">
              <span className="label-text text-xs mb-1">{t("proxy.port")}</span>
              <input
                className="input input-bordered input-sm"
                type="number"
                value={config.proxy.port}
                onChange={(e) =>
                  setConfig({
                    ...config,
                    proxy: { ...config.proxy, port: Number(e.target.value) },
                  })
                }
              />
            </label>
            <label className="form-control">
              <span className="label-text text-xs mb-1">{t("proxy.apiKey")}</span>
              <input
                className="input input-bordered input-sm font-mono"
                type="text"
                value={config.proxy.api_key}
                onChange={(e) =>
                  setConfig({
                    ...config,
                    proxy: { ...config.proxy, api_key: e.target.value },
                  })
                }
              />
            </label>
            <label className="form-control">
              <span className="label-text text-xs mb-1">{t("proxy.authMode")}</span>
              <select
                className="select select-bordered select-sm"
                value={config.proxy.auth_mode}
                onChange={(e) =>
                  setConfig({
                    ...config,
                    proxy: {
                      ...config.proxy,
                      auth_mode: e.target.value as AppConfig["proxy"]["auth_mode"],
                    },
                  })
                }
              >
                <option value="auto">{t("proxy.authAuto")}</option>
                <option value="off">{t("proxy.authOff")}</option>
                <option value="strict">{t("proxy.authStrict")}</option>
                <option value="all_except_health">{t("proxy.authAllExceptHealth")}</option>
              </select>
            </label>
            <label className="form-control">
              <span className="label-text text-xs mb-1">{t("proxy.loadBalanceMode")}</span>
              <select
                className="select select-bordered select-sm"
                value={config.proxy.load_balance_mode ?? "round_robin"}
                onChange={(e) =>
                  setConfig({
                    ...config,
                    proxy: {
                      ...config.proxy,
                      load_balance_mode: e.target.value as AppConfig["proxy"]["load_balance_mode"],
                    },
                  })
                }
              >
                <option value="round_robin">{t("proxy.roundRobin")}</option>
                <option value="failover">{t("proxy.failover")}</option>
                <option value="random">{t("proxy.random")}</option>
                <option value="weighted">{t("proxy.weighted")}</option>
              </select>
            </label>
          </div>
          <div className="flex justify-end">
            <button className="btn btn-primary btn-sm gap-2" onClick={handleSaveConfig}>
              <Save size={14} />
              {t("proxy.saveConfig")}
            </button>
          </div>
        </div>
      </div>

      <div className="card bg-base-100 border border-base-300">
        <div className="card-body gap-4">
          <div className="flex items-start justify-between gap-3 flex-wrap">
            <div>
              <h2 className="card-title text-sm font-medium text-base-content/80">
                <KeyRound size={16} />
                {text("Client API Keys", "Client API Keys")}
              </h2>
              <p className="text-xs text-base-content/50 mt-1">
                {text(
                  "Create separate keys for apps or teammates and limit accessible sites and models.",
                  "Create separate keys for apps or teammates and limit accessible sites and models.",
                )}
              </p>
              <p className="text-xs text-base-content/40 mt-1">
                {text(
                  "When no sites are selected, the key can access all enabled sites.",
                  "When no sites are selected, the key can access all enabled sites.",
                )}
              </p>
            </div>
            <button className="btn btn-primary btn-sm gap-2" onClick={openCreateKey}>
              <Plus size={14} />
              {text("Add Key", "Add Key")}
            </button>
          </div>

          {proxyKeys.length === 0 ? (
            <div className="rounded-lg border border-dashed border-base-300 px-4 py-6 text-sm text-base-content/50">
              {text("No client access keys created yet.", "No client access keys created yet.")}
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="table table-sm">
                <thead>
                  <tr>
                    <th>{text("Label", "Label")}</th>
                    <th>{text("Key", "Key")}</th>
                    <th>{text("Sites", "Sites")}</th>
                    <th>{text("Models", "Models")}</th>
                    <th>{text("Limits", "Limits")}</th>
                    <th>{text("Status", "Status")}</th>
                    <th>{text("Actions", "Actions")}</th>
                  </tr>
                </thead>
                <tbody>
                  {proxyKeys.map((key, index) => (
                    <tr key={`${key.key}-${key.created_at}`}>
                      <td className="font-medium">{key.label || `Key ${index + 1}`}</td>
                      <td>
                        <button
                          className="font-mono text-xs text-primary flex items-center gap-1"
                          onClick={() => copyKey(key.key)}
                          type="button"
                        >
                          {maskKey(key.key)}
                          <Copy size={12} />
                        </button>
                      </td>
                      <td className="text-xs text-base-content/70">
                        {key.allowed_account_ids?.length
                          ? key.allowed_account_ids.length
                          : text("All sites", "All sites")}
                      </td>
                      <td className="text-xs text-base-content/70">
                        {key.allowed_models.length
                          ? key.allowed_models.length
                          : text("All models", "All models")}
                      </td>
                      <td className="text-xs font-mono">
                        D ${key.daily_limit.toFixed(2)} / M ${key.monthly_limit.toFixed(2)}
                      </td>
                      <td>
                        <button
                          className={`badge badge-sm ${key.enabled ? "badge-success" : "badge-error"}`}
                          onClick={() => toggleKeyEnabled(index)}
                          type="button"
                        >
                          {key.enabled ? text("Enabled", "Enabled") : text("Disabled", "Disabled")}
                        </button>
                      </td>
                      <td>
                        <div className="flex items-center gap-2">
                          <button className="btn btn-ghost btn-xs" onClick={() => openEditKey(index)} type="button">
                            <Pencil size={14} />
                          </button>
                          <button
                            className="btn btn-ghost btn-xs text-error"
                            onClick={() => deleteKey(index)}
                            type="button"
                          >
                            <Trash2 size={14} />
                          </button>
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}

          <div className="text-xs text-base-content/40">
            {catalogLoading
              ? text("Loading site/model catalog...", "Loading site/model catalog...")
              : `${text("Configured keys", "Configured keys")}: ${proxyKeys.length}`}
          </div>
        </div>
      </div>

      {status.running && (
        <div className="card bg-base-100 border border-base-300">
          <div className="card-body">
            <h2 className="card-title text-sm font-medium text-base-content/60">
              <Copy size={16} />
              {t("proxy.usageExamples")}
            </h2>
            <div className="code-block">
{`# OpenAI Chat Completions
curl -X POST http://127.0.0.1:${config.proxy.port}/v1/chat/completions \\
  -H "Content-Type: application/json" \\
  -H "Authorization: Bearer ${config.proxy.api_key}" \\
  -d '{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hello"}]}'

# Streaming
curl -X POST http://127.0.0.1:${config.proxy.port}/v1/chat/completions \\
  -H "Content-Type: application/json" \\
  -H "Authorization: Bearer ${config.proxy.api_key}" \\
  -d '{"model":"gpt-4o-mini","stream":true,"messages":[{"role":"user","content":"hello"}]}'

# Health Check
curl http://127.0.0.1:${config.proxy.port}/health`}
            </div>
          </div>
        </div>
      )}

      {status.running && (
        <CliSyncCard
          proxyUrl={`http://127.0.0.1:${config.proxy.port}`}
          apiKey={config.proxy.api_key}
          proxyPort={config.proxy.port}
        />
      )}

      {editorOpen && (
        <div className="fixed inset-0 z-40 flex items-center justify-center bg-black/45 px-4">
          <div className="w-full max-w-4xl rounded-2xl border border-base-300 bg-base-100 shadow-2xl">
            <div className="flex items-center justify-between border-b border-base-300 px-5 py-4">
              <div>
                <h3 className="text-lg font-semibold">
                  {editingIndex == null ? text("Add Key", "Add Key") : text("Edit Key", "Edit Key")}
                </h3>
                <p className="text-sm text-base-content/50 mt-1">
                  {text("Choose sites first, then select models from those sites.", "Choose sites first, then select models from those sites.")}
                </p>
              </div>
              <button className="btn btn-ghost btn-sm" onClick={closeEditor} type="button">
                <X size={16} />
              </button>
            </div>

            <div className="grid grid-cols-1 lg:grid-cols-[1.1fr_0.9fr] gap-5 px-5 py-5">
              <div className="space-y-4">
                <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                  <label className="form-control">
                    <span className="label-text text-xs mb-1">{text("Label", "Label")}</span>
                    <input
                      className="input input-bordered input-sm"
                      value={editor.label}
                      onChange={(e) => setEditor((prev) => ({ ...prev, label: e.target.value }))}
                    />
                  </label>
                  <label className="form-control">
                    <span className="label-text text-xs mb-1">{text("Key", "Key")}</span>
                    <div className="join">
                      <input
                        className="input input-bordered input-sm join-item w-full font-mono"
                        value={editor.key}
                        onChange={(e) => setEditor((prev) => ({ ...prev, key: e.target.value }))}
                      />
                      <button
                        className="btn btn-outline btn-sm join-item"
                        onClick={() => setEditor((prev) => ({ ...prev, key: newKeyValue() }))}
                        type="button"
                      >
                        {text("Reset", "Reset")}
                      </button>
                    </div>
                  </label>
                  <label className="form-control">
                    <span className="label-text text-xs mb-1">{text("Daily limit ($)", "Daily limit ($)")}</span>
                    <input
                      className="input input-bordered input-sm font-mono"
                      type="number"
                      min="0"
                      step="0.01"
                      value={editor.daily_limit}
                      onChange={(e) =>
                        setEditor((prev) => ({ ...prev, daily_limit: Number(e.target.value) || 0 }))
                      }
                    />
                  </label>
                  <label className="form-control">
                    <span className="label-text text-xs mb-1">{text("Monthly limit ($)", "Monthly limit ($)")}</span>
                    <input
                      className="input input-bordered input-sm font-mono"
                      type="number"
                      min="0"
                      step="0.01"
                      value={editor.monthly_limit}
                      onChange={(e) =>
                        setEditor((prev) => ({ ...prev, monthly_limit: Number(e.target.value) || 0 }))
                      }
                    />
                  </label>
                </div>

                <label className="label cursor-pointer justify-start gap-3 rounded-xl border border-base-300 px-4 py-3">
                  <input
                    type="checkbox"
                    className="toggle toggle-primary toggle-sm"
                    checked={editor.enabled}
                    onChange={(e) => setEditor((prev) => ({ ...prev, enabled: e.target.checked }))}
                  />
                  <span className="label-text">{text("Enable key", "Enable key")}</span>
                </label>

                <div className="rounded-xl border border-base-300 p-4">
                  <div className="flex items-center justify-between gap-2 flex-wrap mb-3">
                    <div>
                      <div className="text-sm font-medium">{text("Sites", "Sites")}</div>
                      <div className="text-xs text-base-content/50 mt-1">
                        {text("Selected sites", "Selected sites")}:{" "}
                        {editor.allowed_account_ids.length || text("All sites", "All sites")}
                      </div>
                    </div>
                    <div className="flex items-center gap-2">
                      <button
                        className="btn btn-outline btn-xs"
                        onClick={() =>
                          setEditor((prev) => ({
                            ...prev,
                            allowed_account_ids: siteOptions.map((option) => option.accountId),
                          }))
                        }
                        type="button"
                      >
                        {text("All sites", "All sites")}
                      </button>
                      <button
                        className="btn btn-ghost btn-xs"
                        onClick={() =>
                          setEditor((prev) => ({
                            ...prev,
                            allowed_account_ids: [],
                            allowed_models: prev.allowed_models.filter((model) => availableModels.includes(model)),
                          }))
                        }
                        type="button"
                      >
                        {text("Clear sites", "Clear sites")}
                      </button>
                    </div>
                  </div>
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-2 max-h-64 overflow-y-auto pr-1">
                    {siteOptions.map((option) => (
                      <label
                        key={option.accountId}
                        className="flex items-start gap-3 rounded-lg border border-base-300 px-3 py-2 text-sm"
                      >
                        <input
                          type="checkbox"
                          className="checkbox checkbox-sm mt-0.5"
                          checked={editor.allowed_account_ids.includes(option.accountId)}
                          onChange={() => toggleSite(option.accountId)}
                        />
                        <span className="min-w-0">
                          <span className="block font-medium truncate">{option.label}</span>
                          <span className="block text-xs text-base-content/50 truncate">
                            {option.selector} · {option.modelCount} {text("models", "models")}
                          </span>
                        </span>
                      </label>
                    ))}
                  </div>
                </div>
              </div>

              <div className="rounded-xl border border-base-300 p-4">
                <div className="flex items-center justify-between gap-2 flex-wrap mb-3">
                  <div>
                    <div className="text-sm font-medium">{text("Models", "Models")}</div>
                    <div className="text-xs text-base-content/50 mt-1">
                      {text("Selected models", "Selected models")}:{" "}
                      {editor.allowed_models.length || text("All models", "All models")}
                    </div>
                    <div className="text-xs text-base-content/40 mt-1">
                      {text("Visible results", "Visible results")}: {visibleModels.length}/{availableModels.length}
                    </div>
                  </div>
                  <div className="flex items-center gap-2 flex-wrap">
                    <div className="join">
                      <input
                        className="input input-bordered input-sm join-item w-44 md:w-56"
                        placeholder={text("Search models", "Search models")}
                        value={modelSearchInput}
                        onChange={(e) => setModelSearchInput(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") {
                            applyModelSearch();
                          }
                        }}
                      />
                      <button className="btn btn-outline btn-sm join-item" onClick={applyModelSearch} type="button">
                        <Search size={14} />
                      </button>
                      {(modelSearchInput || modelSearch) && (
                        <button className="btn btn-ghost btn-sm join-item" onClick={clearModelSearch} type="button">
                          <X size={14} />
                        </button>
                      )}
                    </div>
                    <label className="input input-bordered input-sm flex items-center gap-2">
                      <ArrowDownUp size={14} className="text-base-content/50" />
                      <select
                        className="bg-transparent outline-none"
                        value={priceSort}
                        onChange={(e) => setPriceSort(e.target.value as ModelPriceSort)}
                      >
                        <option value="default">{text("默认排序", "Default order")}</option>
                        <option value="asc">{text("价格升序", "Price low-high")}</option>
                        <option value="desc">{text("价格降序", "Price high-low")}</option>
                      </select>
                    </label>
                    <button
                      className="btn btn-outline btn-xs"
                      onClick={() => setEditor((prev) => ({ ...prev, allowed_models: availableModels }))}
                      type="button"
                      disabled={availableModels.length === 0}
                    >
                      {text("All models", "All models")}
                    </button>
                    <button
                      className="btn btn-ghost btn-xs"
                      onClick={() => setEditor((prev) => ({ ...prev, allowed_models: [] }))}
                      type="button"
                    >
                      {text("Clear models", "Clear models")}
                    </button>
                  </div>
                </div>

                {availableModels.length === 0 ? (
                  <div className="rounded-lg border border-dashed border-base-300 px-4 py-6 text-sm text-base-content/50">
                    {text(
                      "No models available yet. Fetch models from your sites first.",
                      "No models available yet. Fetch models from your sites first.",
                    )}
                  </div>
                ) : visibleModels.length === 0 ? (
                  <div className="rounded-lg border border-dashed border-base-300 px-4 py-6 text-sm text-base-content/50">
                    {text("No models matched your search.", "No models matched your search.")}
                  </div>
                ) : (
                  <div className="grid grid-cols-1 gap-2 max-h-[26rem] overflow-y-auto pr-1">
                    {visibleModels.map((model) => (
                      <label
                        key={model}
                        className="flex items-center justify-between gap-3 rounded-lg border border-base-300 px-3 py-2 text-sm font-mono"
                      >
                        <span className="flex min-w-0 flex-1 items-center gap-3">
                          <input
                            type="checkbox"
                            className="checkbox checkbox-sm"
                            checked={editor.allowed_models.includes(model)}
                            onChange={() => toggleModel(model)}
                          />
                          <span
                            className="truncate"
                            onMouseEnter={(event) =>
                              showHoverPreview(
                                event.currentTarget,
                                model,
                                text("完整模型名", "Full model name"),
                              )
                            }
                            onMouseLeave={hideHoverPreview}
                          >
                            {model}
                          </span>
                        </span>
                        <button
                          className="btn btn-ghost btn-xs"
                          type="button"
                          onClick={(e) => {
                            e.preventDefault();
                            e.stopPropagation();
                          }}
                          onMouseEnter={(event) =>
                            showHoverPreview(
                              event.currentTarget,
                              priceTooltip(model),
                              text("价格预览", "Price preview"),
                            )
                          }
                          onMouseLeave={hideHoverPreview}
                          onFocus={(event) =>
                            showHoverPreview(
                              event.currentTarget,
                              priceTooltip(model),
                              text("价格预览", "Price preview"),
                            )
                          }
                          onBlur={hideHoverPreview}
                          aria-label={priceTooltip(model)}
                        >
                          <DollarSign
                            size={14}
                            className={modelPrices[model] ? "text-success" : "text-base-content/30"}
                          />
                        </button>
                      </label>
                    ))}
                  </div>
                )}
              </div>
            </div>

            <div className="flex items-center justify-end gap-3 border-t border-base-300 px-5 py-4">
              <button className="btn btn-ghost" onClick={closeEditor} type="button" disabled={editorBusy}>
                {t("common.cancel")}
              </button>
              <button className="btn btn-primary gap-2" onClick={saveEditor} type="button" disabled={editorBusy}>
                <Check size={16} />
                {text("Save Key", "Save Key")}
              </button>
            </div>
          </div>
        </div>
      )}

      {editorOpen && hoverPreview && (
        <div
          className="pointer-events-none fixed z-[80]"
          style={{
            left: hoverPreview.left,
            top: hoverPreview.top,
            transform:
              hoverPreview.placement === "top"
                ? "translate(-50%, -100%)"
                : "translate(-50%, 0)",
          }}
        >
          {hoverPreview.placement === "bottom" && (
            <div className="mx-auto -mb-1 h-3 w-3 rotate-45 border border-slate-700/90 bg-slate-900/95" />
          )}
          <div className="max-w-[22rem] rounded-2xl border border-slate-700/90 bg-slate-900/95 px-3 py-2 text-left text-xs leading-5 text-slate-100 shadow-2xl backdrop-blur-sm">
            {hoverPreview.title && (
              <div className="mb-1 text-[11px] font-semibold uppercase tracking-[0.14em] text-emerald-300">
                {hoverPreview.title}
              </div>
            )}
            <div className="whitespace-pre-line break-words">{hoverPreview.content}</div>
          </div>
          {hoverPreview.placement === "top" && (
            <div className="mx-auto -mt-1 h-3 w-3 rotate-45 border border-slate-700/90 bg-slate-900/95" />
          )}
        </div>
      )}
    </div>
  );
}
