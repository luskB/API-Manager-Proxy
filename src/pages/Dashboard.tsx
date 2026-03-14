import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { request } from "../utils/request";
import {
  Activity,
  ArrowRight,
  BarChart3,
  Box,
  Clock,
  DollarSign,
  KeyRound,
  Radio,
  TrendingUp,
  UserCheck,
  Users,
  Zap,
} from "lucide-react";
import ErrorAlert from "../components/ErrorAlert";
import PageSkeleton from "../components/PageSkeleton";
import { useConfig } from "../hooks/useConfig";
import { useLocale } from "../hooks/useLocale";

interface ProxyStatus {
  running: boolean;
}

interface AccountStats {
  total_requests: number;
  success_count: number;
  error_count: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_estimated_cost: number;
  total_duration_ms: number;
}

interface WindowKeyStats {
  total_requests: number;
  total_cost: number;
}

interface TimelineBucket {
  timestamp: number;
  total_requests: number;
  success_count: number;
  total_tokens: number;
  total_cost: number;
}

interface ScopedProxyStatsData {
  scope: "hourly" | "daily" | "weekly";
  window_start: number;
  window_end: number;
  global: AccountStats;
  per_account: Record<string, AccountStats>;
  per_model: Record<string, AccountStats>;
  per_key: Record<string, WindowKeyStats>;
  timeline: TimelineBucket[];
}

type StatsScope = "hourly" | "daily" | "weekly";

function StatCard({
  title,
  value,
  icon: Icon,
}: {
  title: string;
  value: string | number;
  icon: React.ElementType;
}) {
  return (
    <div className="flex items-center gap-3 bg-base-100 rounded-lg border border-base-300 px-4 py-2.5">
      <div className="text-primary shrink-0">
        <Icon size={18} />
      </div>
      <div className="min-w-0">
        <div className="text-xs text-base-content/50 leading-tight">{title}</div>
        <div className="text-lg font-semibold leading-tight">{value}</div>
      </div>
    </div>
  );
}

export default function Dashboard() {
  const { config, error, setError, reload } = useConfig();
  const [status, setStatus] = useState<ProxyStatus | null>(null);
  const [models, setModels] = useState<string[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [statsScope, setStatsScope] = useState<StatsScope>("daily");
  const [stats, setStats] = useState<ScopedProxyStatsData | null>(null);
  const { t, locale } = useLocale();

  const text = (zh: string, en: string) => (locale === "zh" ? zh : en);

  useEffect(() => {
    request<ProxyStatus>("get_proxy_status")
      .then(setStatus)
      .catch(() => setStatus({ running: false }));
  }, []);

  useEffect(() => {
    if (!config || config.proxy_accounts.length === 0) {
      setModels([]);
      return;
    }
    setModelsLoading(true);
    request<string[]>("get_available_models")
      .then((items) => setModels(items ?? []))
      .catch(() => setModels([]))
      .finally(() => setModelsLoading(false));
  }, [config?.proxy_accounts]);

  useEffect(() => {
    request<ScopedProxyStatsData>("get_proxy_stats_view", { scope: statsScope })
      .then((payload) => setStats(payload))
      .catch(() => setStats(null));
  }, [statsScope]);

  const scopeOptions: Array<{ value: StatsScope; label: string }> = useMemo(
    () => [
      { value: "hourly", label: text("Hourly", "Hourly") },
      { value: "daily", label: text("Daily", "Daily") },
      { value: "weekly", label: text("Weekly", "Weekly") },
    ],
    [locale],
  );

  const proxyApiKeys = config?.proxy.api_keys ?? [];
  const keyRows = useMemo(() => {
    const configured = proxyApiKeys.map((key) => ({
      id: key.key,
      label: key.label || maskKey(key.key),
      enabled: key.enabled,
      requests: stats?.per_key[key.key]?.total_requests ?? 0,
      cost: stats?.per_key[key.key]?.total_cost ?? 0,
      dailyLimit: key.daily_limit,
      monthlyLimit: key.monthly_limit,
      models: key.allowed_models.length,
      sites: key.allowed_account_ids?.length ?? 0,
    }));
    const configuredIds = new Set(configured.map((row) => row.id));
    const extra = Object.entries(stats?.per_key ?? {})
      .filter(([id]) => !configuredIds.has(id))
      .map(([id, value]) => ({
        id,
        label: maskKey(id),
        enabled: false,
        requests: value.total_requests,
        cost: value.total_cost,
        dailyLimit: 0,
        monthlyLimit: 0,
        models: 0,
        sites: 0,
      }));
    return [...configured, ...extra].sort((a, b) => b.requests - a.requests || b.cost - a.cost);
  }, [proxyApiKeys, stats?.per_key]);

  if (!config) {
    return <PageSkeleton message={t("dashboard.loadingDashboard")} />;
  }

  const totalAccounts = config.accounts.length;
  const proxyAccounts = config.proxy_accounts.length;
  const enabledAccounts = config.proxy_accounts.filter((a) => !a.disabled).length;
  const proxyKeyCount = config.proxy.api_keys?.length ?? 0;
  const QUOTA_CONVERSION_FACTOR = 500000;
  const totalQuota = config.proxy_accounts.reduce(
    (sum, account) => sum + (account.account_info?.quota ?? 0),
    0,
  ) / QUOTA_CONVERSION_FACTOR;

  const successRate = stats?.global.total_requests
    ? ((stats.global.success_count / stats.global.total_requests) * 100).toFixed(1)
    : "0.0";
  const avgLatency = stats?.global.total_requests
    ? Math.round(stats.global.total_duration_ms / stats.global.total_requests)
    : 0;

  function scopeSummary(): string {
    switch (statsScope) {
      case "hourly":
        return text("Last 24 hours", "Last 24 hours");
      case "daily":
        return text("Last 7 days", "Last 7 days");
      case "weekly":
        return text("Last 8 weeks", "Last 8 weeks");
      default:
        return "";
    }
  }

  function formatBucketLabel(timestamp: number): string {
    const date = new Date(timestamp * 1000);
    if (statsScope === "hourly") {
      return String(date.getHours()).padStart(2, "0");
    }
    if (statsScope === "daily") {
      return `${date.getMonth() + 1}/${date.getDate()}`;
    }
    const firstDay = new Date(date);
    firstDay.setDate(date.getDate() - date.getDay());
    return `${firstDay.getMonth() + 1}/${firstDay.getDate()}`;
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">{t("dashboard.title")}</h1>
          <p className="text-base-content/60 text-sm">{t("dashboard.subtitle")}</p>
        </div>
        <div className="flex items-center gap-2">
          <span className={`badge badge-sm ${status?.running ? "badge-success" : "badge-error"}`}>
            {status?.running ? t("common.running") : t("common.stopped")}
          </span>
          {status?.running && <span className="text-xs text-base-content/50">:{config.proxy.port}</span>}
        </div>
      </div>

      {error && (
        <ErrorAlert
          message={error}
          onRetry={() => {
            reload();
          }}
          onDismiss={() => setError("")}
        />
      )}

      {totalAccounts === 0 && (
        <div className="card bg-base-100 border border-base-300">
          <div className="card-body items-center text-center py-8">
            <Users size={36} className="text-base-content/20 mb-1" />
            <h2 className="card-title text-base">{t("dashboard.getStarted")}</h2>
            <p className="text-base-content/60 text-sm max-w-md">{t("dashboard.getStartedDesc")}</p>
            <div className="card-actions mt-3">
              <Link to="/accounts" className="btn btn-primary btn-sm gap-2">
                {t("dashboard.goToAccounts")}
                <ArrowRight size={14} />
              </Link>
            </div>
          </div>
        </div>
      )}

      <div className="grid grid-cols-2 lg:grid-cols-5 gap-2">
        <StatCard title={t("dashboard.totalAccounts")} value={totalAccounts} icon={Users} />
        <StatCard title={t("dashboard.proxyAccounts")} value={proxyAccounts} icon={Radio} />
        <StatCard title={t("dashboard.activeAccounts")} value={enabledAccounts} icon={UserCheck} />
        <StatCard title={t("dashboard.totalQuota")} value={`$${totalQuota.toFixed(2)}`} icon={DollarSign} />
        <StatCard title={text("Client Keys", "Client Keys")} value={proxyKeyCount} icon={KeyRound} />
      </div>

      <div className="card bg-base-100 border border-base-300">
        <div className="card-body gap-4">
          <div className="flex items-center justify-between gap-3 flex-wrap">
            <div>
              <h2 className="card-title text-sm font-medium text-base-content/70">
                <Activity size={16} />
                {text("Stats Window", "Stats Window")}
              </h2>
              <p className="text-xs text-base-content/50 mt-1">{scopeSummary()}</p>
            </div>
            <div className="join">
              {scopeOptions.map((option) => (
                <button
                  key={option.value}
                  className={`btn btn-sm join-item ${statsScope === option.value ? "btn-primary" : "btn-outline"}`}
                  onClick={() => setStatsScope(option.value)}
                  type="button"
                >
                  {option.label}
                </button>
              ))}
            </div>
          </div>

          <div className="grid grid-cols-2 lg:grid-cols-4 gap-2">
            <StatCard title={t("dashboard.totalRequests")} value={stats?.global.total_requests ?? 0} icon={Activity} />
            <StatCard
              title={t("dashboard.estimatedCost")}
              value={`$${(stats?.global.total_estimated_cost ?? 0).toFixed(4)}`}
              icon={DollarSign}
            />
            <StatCard title={t("dashboard.successRate")} value={`${successRate}%`} icon={TrendingUp} />
            <StatCard title={t("dashboard.avgLatency")} value={`${avgLatency}ms`} icon={Clock} />
          </div>
        </div>
      </div>

      {stats && stats.timeline.length > 0 && (
        <div className="card bg-base-100 border border-base-300">
          <div className="card-body">
            <h2 className="card-title text-sm font-medium text-base-content/60">
              <TrendingUp size={16} />
              {text("Request Trend", "Request Trend")}
            </h2>
            <div className="flex items-end gap-1 h-32 mt-2">
              {(() => {
                const maxReqs = Math.max(...stats.timeline.map((bucket) => bucket.total_requests), 1);
                return stats.timeline.map((bucket) => {
                  const height = Math.max((bucket.total_requests / maxReqs) * 100, 2);
                  const successPct = bucket.total_requests
                    ? (bucket.success_count / bucket.total_requests) * 100
                    : 100;
                  const barColor =
                    successPct >= 90 ? "bg-success" : successPct >= 50 ? "bg-warning" : "bg-error";
                  return (
                    <div key={bucket.timestamp} className="flex flex-col items-center flex-1 min-w-0">
                      <div
                        className="tooltip tooltip-top w-full"
                        data-tip={`${bucket.total_requests} reqs, $${bucket.total_cost.toFixed(4)}`}
                      >
                        <div
                          className={`w-full rounded-t ${barColor}`}
                          style={{ height: `${height}%`, minHeight: "2px" }}
                        />
                      </div>
                      <span className="text-[9px] text-base-content/40 mt-1">
                        {formatBucketLabel(bucket.timestamp)}
                      </span>
                    </div>
                  );
                });
              })()}
            </div>
          </div>
        </div>
      )}

      {stats && Object.keys(stats.per_account).length > 0 && (
        <div className="card bg-base-100 border border-base-300">
          <div className="card-body">
            <h2 className="card-title text-sm font-medium text-base-content/60">
              <Activity size={16} />
              {text("Site Stats", "Site Stats")}
            </h2>
            <div className="overflow-x-auto">
              <table className="table table-sm">
                <thead>
                  <tr>
                    <th>{t("table.account")}</th>
                    <th>{t("table.requests")}</th>
                    <th>{t("table.success")}</th>
                    <th>{t("table.errors")}</th>
                    <th>{t("table.tokens")}</th>
                    <th>{t("table.cost")}</th>
                    <th>{t("table.avgLatency")}</th>
                  </tr>
                </thead>
                <tbody>
                  {Object.entries(stats.per_account)
                    .sort(([, left], [, right]) => right.total_requests - left.total_requests)
                    .map(([id, value]) => {
                      const account = config.proxy_accounts.find((item) => item.id === id);
                      const label = account
                        ? `${account.site_name} (${account.account_info.username})`
                        : id;
                      const latency = value.total_requests
                        ? Math.round(value.total_duration_ms / value.total_requests)
                        : 0;
                      return (
                        <tr key={id}>
                          <td className="text-xs max-w-[220px] truncate" title={id}>
                            {label}
                          </td>
                          <td>{value.total_requests}</td>
                          <td className="text-success">{value.success_count}</td>
                          <td className="text-error">{value.error_count}</td>
                          <td className="font-mono text-xs">
                            {value.total_input_tokens}/{value.total_output_tokens}
                          </td>
                          <td className="font-mono text-xs">${value.total_estimated_cost.toFixed(4)}</td>
                          <td className="font-mono text-xs">{latency}ms</td>
                        </tr>
                      );
                    })}
                </tbody>
              </table>
            </div>
          </div>
        </div>
      )}

      {stats && Object.keys(stats.per_model).length > 0 && (
        <div className="card bg-base-100 border border-base-300">
          <div className="card-body">
            <h2 className="card-title text-sm font-medium text-base-content/60">
              <BarChart3 size={16} />
              {t("dashboard.topModels")}
            </h2>
            <div className="overflow-x-auto">
              <table className="table table-sm">
                <thead>
                  <tr>
                    <th>{t("table.model")}</th>
                    <th>{t("table.requests")}</th>
                    <th>{t("table.success")}</th>
                    <th>{t("table.errors")}</th>
                    <th>{t("table.tokens")}</th>
                    <th>{t("table.cost")}</th>
                    <th>{t("table.avgLatency")}</th>
                  </tr>
                </thead>
                <tbody>
                  {Object.entries(stats.per_model)
                    .sort(([, left], [, right]) => right.total_requests - left.total_requests)
                    .slice(0, 12)
                    .map(([model, value]) => {
                      const latency = value.total_requests
                        ? Math.round(value.total_duration_ms / value.total_requests)
                        : 0;
                      return (
                        <tr key={model}>
                          <td className="text-xs font-mono">{model}</td>
                          <td>{value.total_requests}</td>
                          <td className="text-success">{value.success_count}</td>
                          <td className="text-error">{value.error_count}</td>
                          <td className="font-mono text-xs">
                            {value.total_input_tokens}/{value.total_output_tokens}
                          </td>
                          <td className="font-mono text-xs">${value.total_estimated_cost.toFixed(4)}</td>
                          <td className="font-mono text-xs">{latency}ms</td>
                        </tr>
                      );
                    })}
                </tbody>
              </table>
            </div>
          </div>
        </div>
      )}

      {keyRows.length > 0 && (
        <div className="card bg-base-100 border border-base-300">
          <div className="card-body">
            <h2 className="card-title text-sm font-medium text-base-content/60">
              <KeyRound size={16} />
              {text("Client Key Stats", "Client Key Stats")}
            </h2>
            <div className="overflow-x-auto">
              <table className="table table-sm">
                <thead>
                  <tr>
                    <th>{text("Label", "Label")}</th>
                    <th>{t("table.requests")}</th>
                    <th>{t("table.cost")}</th>
                    <th>{text("Sites", "Sites")}</th>
                    <th>{text("Models", "Models")}</th>
                    <th>{text("Limits", "Limits")}</th>
                    <th>{text("Status", "Status")}</th>
                  </tr>
                </thead>
                <tbody>
                  {keyRows.map((row) => (
                    <tr key={row.id}>
                      <td className="text-xs max-w-[220px] truncate" title={row.id}>
                        {row.label}
                      </td>
                      <td>{row.requests}</td>
                      <td className="font-mono text-xs">${row.cost.toFixed(4)}</td>
                      <td>{row.sites || text("All", "All")}</td>
                      <td>{row.models || text("All", "All")}</td>
                      <td className="font-mono text-xs">
                        D ${row.dailyLimit.toFixed(2)} / M ${row.monthlyLimit.toFixed(2)}
                      </td>
                      <td>
                        <span className={`badge badge-sm ${row.enabled ? "badge-success" : "badge-ghost"}`}>
                          {row.enabled ? text("Enabled", "Enabled") : text("Inactive", "Inactive")}
                        </span>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </div>
      )}

      <div className="card bg-base-100 border border-base-300">
        <div className="card-body">
          <h2 className="card-title text-sm font-medium text-base-content/60">
            <Box size={16} />
            {t("dashboard.availableModels")}
          </h2>
          {modelsLoading ? (
            <p className="text-sm text-base-content/40">{t("dashboard.loadingModels")}</p>
          ) : models.length > 0 ? (
            <div className="flex flex-wrap gap-1.5">
              {models.map((model) => (
                <span key={model} className="badge badge-sm badge-outline">
                  {model}
                </span>
              ))}
            </div>
          ) : (
            <p className="text-sm text-base-content/40">{t("dashboard.noModels")}</p>
          )}
        </div>
      </div>

      <div className="card bg-base-100 border border-base-300">
        <div className="card-body">
          <h2 className="card-title text-sm font-medium text-base-content/60">
            <Zap size={16} />
            {t("dashboard.quickStart")}
          </h2>
          <p className="text-sm text-base-content/60 mb-2">{t("dashboard.quickStartDesc")}</p>
          <div className="code-block">
{`# OpenAI Compatible
curl http://127.0.0.1:${config.proxy.port}/v1/chat/completions \\
  -H "Authorization: Bearer ${config.proxy.api_key}" \\
  -H "Content-Type: application/json" \\
  -d '{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hi"}]}'

# Anthropic Compatible
curl http://127.0.0.1:${config.proxy.port}/v1/messages \\
  -H "Authorization: Bearer ${config.proxy.api_key}" \\
  -H "anthropic-version: 2023-06-01" \\
  -H "Content-Type: application/json" \\
  -d '{"model":"claude-3-haiku","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}'`}
          </div>
        </div>
      </div>
    </div>
  );
}

function maskKey(value: string): string {
  if (!value) return "-";
  if (value.length <= 14) return value;
  return `${value.slice(0, 8)}...${value.slice(-4)}`;
}
