import { useEffect, useMemo, useState } from "react";
import {
  Activity,
  ArrowDownCircle,
  ArrowUpCircle,
  Coins,
  Database,
  DollarSign,
} from "lucide-react";
import PageSkeleton from "../components/PageSkeleton";
import ErrorAlert from "../components/ErrorAlert";
import { useConfig } from "../hooks/useConfig";
import { useLocale } from "../hooks/useLocale";
import { request } from "../utils/request";

type StatsScope = "hourly" | "daily" | "weekly";

interface AccountStats {
  total_requests: number;
  success_count: number;
  error_count: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_estimated_cost: number;
  total_duration_ms: number;
}

interface TokenTimelineBucket {
  timestamp: number;
  total_requests: number;
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  total_cost: number;
}

interface TokenStatsView {
  scope: StatsScope;
  window_start: number;
  window_end: number;
  summary: AccountStats;
  per_account: Record<string, AccountStats>;
  per_model: Record<string, AccountStats>;
  timeline: TokenTimelineBucket[];
}

function formatTokenCount(value: number): string {
  return Math.round(value).toLocaleString();
}

function formatUsd(value: number): string {
  if (value >= 100) return value.toFixed(2);
  if (value >= 1) return value.toFixed(3);
  if (value >= 0.01) return value.toFixed(4);
  return value.toFixed(6);
}

function StatCard({
  title,
  value,
  icon: Icon,
  hint,
}: {
  title: string;
  value: string | number;
  icon: React.ElementType;
  hint?: string;
}) {
  return (
    <div className="flex items-center gap-3 rounded-lg border border-base-300 bg-base-100 px-4 py-3">
      <div className="shrink-0 text-primary">
        <Icon size={18} />
      </div>
      <div className="min-w-0">
        <div className="text-xs text-base-content/50">{title}</div>
        <div className="text-lg font-semibold">{value}</div>
        {hint && <div className="text-[11px] text-base-content/45">{hint}</div>}
      </div>
    </div>
  );
}

export default function TokenStatsPage() {
  const { config, error, setError, reload } = useConfig();
  const [scope, setScope] = useState<StatsScope>("daily");
  const [stats, setStats] = useState<TokenStatsView | null>(null);
  const [loading, setLoading] = useState(false);
  const { locale } = useLocale();

  const text = (zh: string, en: string) => (locale === "zh" ? zh : en);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    request<TokenStatsView>("get_token_stats_view", { scope })
      .then((payload) => {
        if (!cancelled) {
          setStats(payload);
        }
      })
      .catch((requestError) => {
        if (!cancelled) {
          setStats(null);
          setError(String(requestError));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [scope]);

  const scopeOptions = useMemo(
    () => [
      { value: "hourly" as const, label: text("按小时", "Hourly") },
      { value: "daily" as const, label: text("按天", "Daily") },
      { value: "weekly" as const, label: text("按周", "Weekly") },
    ],
    [locale],
  );

  const accountRows = useMemo(() => {
    if (!stats) return [];
    return Object.entries(stats.per_account).sort(
      ([, left], [, right]) =>
        right.total_input_tokens + right.total_output_tokens - (left.total_input_tokens + left.total_output_tokens),
    );
  }, [stats]);

  const modelRows = useMemo(() => {
    if (!stats) return [];
    return Object.entries(stats.per_model).sort(
      ([, left], [, right]) =>
        right.total_input_tokens + right.total_output_tokens - (left.total_input_tokens + left.total_output_tokens),
    );
  }, [stats]);

  if (!config) {
    return <PageSkeleton />;
  }

  const totalTokens =
    (stats?.summary.total_input_tokens ?? 0) + (stats?.summary.total_output_tokens ?? 0);
  const avgPerRequest = stats?.summary.total_requests
    ? totalTokens / stats.summary.total_requests
    : 0;

  function scopeHint(): string {
    switch (scope) {
      case "hourly":
        return text("最近 24 小时", "Last 24 hours");
      case "daily":
        return text("最近 7 天", "Last 7 days");
      case "weekly":
        return text("最近 8 周", "Last 8 weeks");
      default:
        return "";
    }
  }

  function formatBucketLabel(timestamp: number): string {
    const date = new Date(timestamp * 1000);
    if (scope === "hourly") {
      return `${String(date.getHours()).padStart(2, "0")}:00`;
    }
    if (scope === "daily") {
      return `${date.getMonth() + 1}/${date.getDate()}`;
    }
    return `${date.getMonth() + 1}/${date.getDate()}`;
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3 flex-wrap">
        <div>
          <h1 className="text-2xl font-bold">{text("Token统计", "Token Stats")}</h1>
          <p className="text-sm text-base-content/60">
            {text("聚合查看输入、输出、总 Token 与费用趋势。", "Track input, output, total tokens, and cost trends.")}
          </p>
        </div>
        <div className="join">
          {scopeOptions.map((option) => (
            <button
              key={option.value}
              type="button"
              className={`btn btn-sm join-item ${scope === option.value ? "btn-primary" : "btn-outline"}`}
              onClick={() => setScope(option.value)}
            >
              {option.label}
            </button>
          ))}
        </div>
      </div>

      {error && (
        <ErrorAlert
          message={error}
          onRetry={() => reload()}
          onDismiss={() => setError("")}
        />
      )}

      {loading && !stats ? (
        <PageSkeleton />
      ) : (
        <>
          <div className="rounded-lg border border-base-300 bg-base-100 p-4">
            <div className="mb-4 flex items-center justify-between gap-3 flex-wrap">
              <div>
                <h2 className="text-sm font-medium text-base-content/70">{text("统计窗口", "Window")}</h2>
                <p className="text-xs text-base-content/50">{scopeHint()}</p>
              </div>
              <div className="badge badge-outline">{scopeHint()}</div>
            </div>
            <div className="grid grid-cols-2 lg:grid-cols-6 gap-2">
              <StatCard
                title={text("请求数", "Requests")}
                value={stats?.summary.total_requests ?? 0}
                icon={Activity}
              />
              <StatCard
                title={text("输入Token", "Input Tokens")}
                value={formatTokenCount(stats?.summary.total_input_tokens ?? 0)}
                icon={ArrowDownCircle}
              />
              <StatCard
                title={text("输出Token", "Output Tokens")}
                value={formatTokenCount(stats?.summary.total_output_tokens ?? 0)}
                icon={ArrowUpCircle}
              />
              <StatCard
                title={text("总Token", "Total Tokens")}
                value={formatTokenCount(totalTokens)}
                icon={Coins}
              />
              <StatCard
                title={text("单次均值", "Avg / Request")}
                value={formatTokenCount(avgPerRequest)}
                icon={Database}
              />
              <StatCard
                title={text("预估费用", "Estimated Cost")}
                value={`$${formatUsd(stats?.summary.total_estimated_cost ?? 0)}`}
                icon={DollarSign}
              />
            </div>
          </div>

          <div className="rounded-lg border border-base-300 bg-base-100 p-4">
            <div className="mb-3">
              <h2 className="text-sm font-medium text-base-content/70">
                {text("Token趋势", "Token Trend")}
              </h2>
              <p className="text-xs text-base-content/50">
                {text("柱状图按时间窗口展示输入与输出 Token，总高度代表总量。", "Bars show input and output tokens over time; total height reflects total volume.")}
              </p>
            </div>
            {!stats || stats.timeline.length === 0 ? (
              <div className="rounded-lg border border-dashed border-base-300 px-4 py-8 text-center text-sm text-base-content/50">
                {text("暂无可用 Token 统计。", "No token stats yet.")}
              </div>
            ) : (
              <div className="flex h-40 items-end gap-2 overflow-x-auto">
                {(() => {
                  const maxTokens = Math.max(...stats.timeline.map((bucket) => bucket.total_tokens), 1);
                  return stats.timeline.map((bucket) => {
                    const totalHeight = Math.max((bucket.total_tokens / maxTokens) * 100, 3);
                    const inputRatio = bucket.total_tokens > 0 ? bucket.input_tokens / bucket.total_tokens : 0;
                    const inputHeight = totalHeight * inputRatio;
                    const outputHeight = totalHeight - inputHeight;
                    return (
                      <div key={bucket.timestamp} className="flex min-w-[3rem] flex-1 flex-col items-center">
                        <div
                          className="tooltip tooltip-top flex h-28 w-full items-end"
                          data-tip={`${formatTokenCount(bucket.input_tokens)} in / ${formatTokenCount(bucket.output_tokens)} out · $${formatUsd(bucket.total_cost)}`}
                        >
                          <div className="flex w-full flex-col justify-end rounded-t border border-base-300/70 bg-base-200/40 overflow-hidden">
                            <div
                              className="w-full bg-primary/70"
                              style={{ height: `${Math.max(inputHeight, bucket.input_tokens > 0 ? 2 : 0)}%` }}
                            />
                            <div
                              className="w-full bg-secondary/70"
                              style={{ height: `${Math.max(outputHeight, bucket.output_tokens > 0 ? 2 : 0)}%` }}
                            />
                          </div>
                        </div>
                        <span className="mt-2 text-[10px] text-base-content/45">
                          {formatBucketLabel(bucket.timestamp)}
                        </span>
                      </div>
                    );
                  });
                })()}
              </div>
            )}
          </div>

          <div className="grid grid-cols-1 xl:grid-cols-2 gap-4">
            <div className="rounded-lg border border-base-300 bg-base-100 p-4">
              <div className="mb-3">
                <h2 className="text-sm font-medium text-base-content/70">
                  {text("热门模型", "Top Models")}
                </h2>
              </div>
              <div className="overflow-x-auto">
                <table className="table table-sm">
                  <thead>
                    <tr>
                      <th>{text("模型", "Model")}</th>
                      <th>{text("请求", "Requests")}</th>
                      <th>{text("输入", "Input")}</th>
                      <th>{text("输出", "Output")}</th>
                      <th>{text("费用", "Cost")}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {modelRows.slice(0, 12).map(([model, row]) => (
                      <tr key={model}>
                        <td className="max-w-[18rem] truncate font-mono text-xs">{model}</td>
                        <td>{row.total_requests}</td>
                        <td>{formatTokenCount(row.total_input_tokens)}</td>
                        <td>{formatTokenCount(row.total_output_tokens)}</td>
                        <td className="font-mono text-xs">${formatUsd(row.total_estimated_cost)}</td>
                      </tr>
                    ))}
                    {modelRows.length === 0 && (
                      <tr>
                        <td colSpan={5} className="py-6 text-center text-sm text-base-content/50">
                          {text("暂无模型统计。", "No model stats yet.")}
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </div>

            <div className="rounded-lg border border-base-300 bg-base-100 p-4">
              <div className="mb-3">
                <h2 className="text-sm font-medium text-base-content/70">
                  {text("站点消耗", "Site Usage")}
                </h2>
              </div>
              <div className="overflow-x-auto">
                <table className="table table-sm">
                  <thead>
                    <tr>
                      <th>{text("站点", "Site")}</th>
                      <th>{text("请求", "Requests")}</th>
                      <th>{text("输入", "Input")}</th>
                      <th>{text("输出", "Output")}</th>
                      <th>{text("费用", "Cost")}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {accountRows.slice(0, 12).map(([accountId, row]) => {
                      const account = config.proxy_accounts.find((item) => item.id === accountId);
                      const label = account
                        ? `${account.site_name}${account.account_info.username ? ` (${account.account_info.username})` : ""}`
                        : accountId;
                      return (
                        <tr key={accountId}>
                          <td className="max-w-[16rem] truncate">{label}</td>
                          <td>{row.total_requests}</td>
                          <td>{formatTokenCount(row.total_input_tokens)}</td>
                          <td>{formatTokenCount(row.total_output_tokens)}</td>
                          <td className="font-mono text-xs">${formatUsd(row.total_estimated_cost)}</td>
                        </tr>
                      );
                    })}
                    {accountRows.length === 0 && (
                      <tr>
                        <td colSpan={5} className="py-6 text-center text-sm text-base-content/50">
                          {text("暂无站点统计。", "No site stats yet.")}
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
