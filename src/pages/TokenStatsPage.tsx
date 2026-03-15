import { useEffect, useMemo, useState } from "react";
import {
  Activity,
  ArrowDownCircle,
  ArrowUpCircle,
  BarChart3,
  Coins,
  DollarSign,
  Layers3,
  Server,
} from "lucide-react";
import PageSkeleton from "../components/PageSkeleton";
import ErrorAlert from "../components/ErrorAlert";
import { useConfig } from "../hooks/useConfig";
import { useLocale } from "../hooks/useLocale";
import { request } from "../utils/request";

type StatsScope = "hourly" | "daily" | "weekly";
type DistributionMetric = "cost" | "tokens";

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

interface TokenModelSummary {
  model: string;
  total_cost: number;
  total_input_tokens: number;
  total_output_tokens: number;
  total_requests: number;
}

interface TokenModelDistributionSegment {
  model: string;
  cost: number;
  total_tokens: number;
}

interface TokenModelDistributionBucket {
  timestamp: number;
  total_cost: number;
  total_tokens: number;
  segments: TokenModelDistributionSegment[];
}

interface TokenStatsView {
  scope: StatsScope;
  window_start: number;
  window_end: number;
  summary: AccountStats;
  per_account: Record<string, AccountStats>;
  per_model: Record<string, AccountStats>;
  timeline: TokenTimelineBucket[];
  top_models: TokenModelSummary[];
  distribution: TokenModelDistributionBucket[];
}

const DISTRIBUTION_COLORS = [
  "#60A5FA",
  "#FDE68A",
  "#34D399",
  "#F472B6",
  "#A78BFA",
  "#94A3B8",
];

function formatTokenCount(value: number): string {
  return Math.round(value).toLocaleString();
}

function formatCompactNumber(value: number): string {
  const absolute = Math.abs(value);
  if (absolute >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(absolute >= 10_000_000 ? 0 : 1)}M`;
  }
  if (absolute >= 1_000) {
    return `${(value / 1_000).toFixed(absolute >= 10_000 ? 0 : 1)}K`;
  }
  return Math.round(value).toString();
}

function formatRate(value: number): string {
  if (value >= 1000) {
    return value.toLocaleString(undefined, {
      maximumFractionDigits: 1,
      minimumFractionDigits: 0,
    });
  }
  if (value >= 100) {
    return value.toFixed(1);
  }
  if (value >= 10) {
    return value.toFixed(2);
  }
  return value.toFixed(3);
}

function formatUsd(value: number): string {
  if (value >= 100) return value.toFixed(2);
  if (value >= 1) return value.toFixed(3);
  if (value >= 0.01) return value.toFixed(4);
  return value.toFixed(6);
}

function bucketStepSeconds(scope: StatsScope): number {
  switch (scope) {
    case "hourly":
      return 5 * 60;
    case "daily":
      return 60 * 60;
    case "weekly":
      return 24 * 60 * 60;
    default:
      return 24 * 60 * 60;
  }
}

function scopeWindowMinutes(scope: StatsScope): number {
  switch (scope) {
    case "hourly":
      return 60;
    case "daily":
      return 24 * 60;
    case "weekly":
      return 7 * 24 * 60;
    default:
      return 24 * 60;
  }
}

function alignTimestamp(timestamp: number, step: number): number {
  return Math.floor(timestamp / step) * step;
}

function metricValue(summary: TokenModelSummary, mode: DistributionMetric): number {
  if (mode === "cost") {
    return summary.total_cost;
  }
  return summary.total_input_tokens + summary.total_output_tokens;
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
    <div className="rounded-2xl border border-base-300 bg-base-100 px-4 py-4 shadow-sm">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-xs uppercase tracking-[0.16em] text-base-content/45">{title}</div>
          <div className="mt-2 text-xl font-semibold text-base-content">{value}</div>
          {hint && <div className="mt-1 text-xs text-base-content/45">{hint}</div>}
        </div>
        <div className="rounded-2xl bg-primary/10 p-2 text-primary">
          <Icon size={18} />
        </div>
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
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";

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
  }, [scope, setError]);

  const scopeOptions = useMemo(
    () => [
      { value: "hourly" as const, label: text("按小时", "Hourly") },
      { value: "daily" as const, label: text("按天", "Daily") },
      { value: "weekly" as const, label: text("按周", "Weekly") },
    ],
    [locale],
  );

  const totalTokens =
    (stats?.summary.total_input_tokens ?? 0) + (stats?.summary.total_output_tokens ?? 0);
  const windowMinutes = scopeWindowMinutes(scope);
  const averageRpm = (stats?.summary.total_requests ?? 0) / windowMinutes;
  const averageTpm = totalTokens / windowMinutes;

  const distributionMetric = useMemo<DistributionMetric>(() => {
    if (!stats) return "tokens";
    const hasCost =
      stats.top_models.some((row) => row.total_cost > 0) ||
      stats.distribution.some((bucket) => bucket.total_cost > 0);
    return hasCost ? "cost" : "tokens";
  }, [stats]);

  const accountRows = useMemo(() => {
    if (!stats) return [];
    return Object.entries(stats.per_account).sort(([, left], [, right]) => {
      if (right.total_estimated_cost !== left.total_estimated_cost) {
        return right.total_estimated_cost - left.total_estimated_cost;
      }
      return (
        right.total_input_tokens +
        right.total_output_tokens -
        (left.total_input_tokens + left.total_output_tokens)
      );
    });
  }, [stats]);

  const modelRows = useMemo(() => {
    if (!stats) return [];
    return Object.entries(stats.per_model).sort(([, left], [, right]) => {
      if (right.total_estimated_cost !== left.total_estimated_cost) {
        return right.total_estimated_cost - left.total_estimated_cost;
      }
      return (
        right.total_input_tokens +
        right.total_output_tokens -
        (left.total_input_tokens + left.total_output_tokens)
      );
    });
  }, [stats]);

  const featuredModels = useMemo<TokenModelSummary[]>(() => {
    if (!stats) return [];
    if (stats.top_models.length > 0) {
      return stats.top_models.slice(0, 5);
    }
    return modelRows.slice(0, 5).map(([model, row]) => ({
      model,
      total_cost: row.total_estimated_cost,
      total_input_tokens: row.total_input_tokens,
      total_output_tokens: row.total_output_tokens,
      total_requests: row.total_requests,
    }));
  }, [modelRows, stats]);

  const colorByModel = useMemo(() => {
    const colorMap = new Map<string, string>();
    featuredModels.forEach((row, index) => {
      colorMap.set(row.model, DISTRIBUTION_COLORS[index % DISTRIBUTION_COLORS.length]);
    });
    colorMap.set("Other", "#94A3B8");
    return colorMap;
  }, [featuredModels]);

  const distributionBuckets = useMemo<TokenModelDistributionBucket[]>(() => {
    if (!stats) return [];

    const step = bucketStepSeconds(scope);
    const start = alignTimestamp(stats.window_start, step);
    const end = alignTimestamp(stats.window_end, step);
    const source = new Map(stats.distribution.map((bucket) => [bucket.timestamp, bucket]));
    const orderedModels = featuredModels.map((row) => row.model);
    const rows: TokenModelDistributionBucket[] = [];

    for (let timestamp = start; timestamp <= end; timestamp += step) {
      const bucket = source.get(timestamp) ?? {
        timestamp,
        total_cost: 0,
        total_tokens: 0,
        segments: [],
      };

      const segmentMap = new Map(bucket.segments.map((segment) => [segment.model, segment]));
      const orderedSegments: TokenModelDistributionSegment[] = orderedModels.map((model) => {
        const segment = segmentMap.get(model);
        return {
          model,
          cost: segment?.cost ?? 0,
          total_tokens: segment?.total_tokens ?? 0,
        };
      });

      for (const [model, segment] of segmentMap.entries()) {
        if (!orderedModels.includes(model)) {
          orderedSegments.push({
            model,
            cost: segment.cost,
            total_tokens: segment.total_tokens,
          });
        }
      }

      rows.push({
        timestamp,
        total_cost: bucket.total_cost,
        total_tokens: bucket.total_tokens,
        segments: orderedSegments,
      });
    }

    return rows;
  }, [featuredModels, scope, stats]);

  const maxDistributionValue = useMemo(() => {
    const values = distributionBuckets.map((bucket) =>
      distributionMetric === "cost" ? bucket.total_cost : bucket.total_tokens,
    );
    return Math.max(...values, 1);
  }, [distributionBuckets, distributionMetric]);

  const modelRankingRows = useMemo(() => {
    return featuredModels.length > 0
      ? featuredModels
      : modelRows.slice(0, 5).map(([model, row]) => ({
          model,
          total_cost: row.total_estimated_cost,
          total_input_tokens: row.total_input_tokens,
          total_output_tokens: row.total_output_tokens,
          total_requests: row.total_requests,
        }));
  }, [featuredModels, modelRows]);

  const maxModelMetricValue = useMemo(
    () => Math.max(...modelRankingRows.map((row) => metricValue(row, distributionMetric)), 1),
    [distributionMetric, modelRankingRows],
  );

  const maxAccountCost = useMemo(
    () => Math.max(...accountRows.map(([, row]) => row.total_estimated_cost), 1),
    [accountRows],
  );

  function scopeHint(): string {
    switch (scope) {
      case "hourly":
        return text("最近 60 分钟", "Last 60 minutes");
      case "daily":
        return text("最近 24 小时", "Last 24 hours");
      case "weekly":
        return text("最近 7 天", "Last 7 days");
      default:
        return "";
    }
  }

  function formatBucketLabel(timestamp: number): string {
    const date = new Date(timestamp * 1000);
    if (scope === "hourly" || scope === "daily") {
      return new Intl.DateTimeFormat(localeTag, {
        hour: "2-digit",
        minute: "2-digit",
      }).format(date);
    }
    return new Intl.DateTimeFormat(localeTag, {
      month: "2-digit",
      day: "2-digit",
    }).format(date);
  }

  function formatDistributionValue(value: number): string {
    if (distributionMetric === "cost") {
      return `$${formatUsd(value)}`;
    }
    return formatCompactNumber(value);
  }

  function chartTooltip(bucket: TokenModelDistributionBucket): string {
    const totalLabel =
      distributionMetric === "cost"
        ? text(`总费用 $${formatUsd(bucket.total_cost)}`, `Total cost $${formatUsd(bucket.total_cost)}`)
        : text(
            `总 Token ${formatTokenCount(bucket.total_tokens)}`,
            `Total tokens ${formatTokenCount(bucket.total_tokens)}`,
          );

    const segmentLines = bucket.segments
      .filter((segment) => (distributionMetric === "cost" ? segment.cost : segment.total_tokens) > 0)
      .map((segment) =>
        distributionMetric === "cost"
          ? `${segment.model}: $${formatUsd(segment.cost)}`
          : `${segment.model}: ${formatTokenCount(segment.total_tokens)}`,
      );

    return [formatBucketLabel(bucket.timestamp), totalLabel, ...segmentLines].join("\n");
  }

  if (!config) {
    return <PageSkeleton />;
  }

  return (
    <div className="space-y-5">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-2xl font-bold">{text("Token 统计", "Token Stats")}</h1>
          <p className="mt-1 text-sm text-base-content/60">
            {text(
              "更清晰地查看模型、站点与时间窗口内的 Token 和费用消耗。",
              "Track model, site, token, and cost usage with a clearer distribution view.",
            )}
          </p>
        </div>
        <div className="join rounded-2xl bg-base-100 p-1 shadow-sm">
          {scopeOptions.map((option) => (
            <button
              key={option.value}
              type="button"
              className={`btn btn-sm join-item border-0 ${scope === option.value ? "btn-primary" : "btn-ghost"}`}
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
          <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
            <StatCard
              title={text("请求数", "Requests")}
              value={stats?.summary.total_requests ?? 0}
              icon={Activity}
              hint={scopeHint()}
            />
            <StatCard
              title={text("总 Token", "Total Tokens")}
              value={formatTokenCount(totalTokens)}
              icon={Coins}
            />
            <StatCard
              title={text("输入 Token", "Input Tokens")}
              value={formatTokenCount(stats?.summary.total_input_tokens ?? 0)}
              icon={ArrowDownCircle}
            />
            <StatCard
              title={text("输出 Token", "Output Tokens")}
              value={formatTokenCount(stats?.summary.total_output_tokens ?? 0)}
              icon={ArrowUpCircle}
            />
            <StatCard
              title={text("平均每次", "Avg / Request")}
              value={formatTokenCount(
                stats?.summary.total_requests ? totalTokens / stats.summary.total_requests : 0,
              )}
              icon={Layers3}
            />
            <StatCard
              title={text("预估费用", "Estimated Cost")}
              value={`$${formatUsd(stats?.summary.total_estimated_cost ?? 0)}`}
              icon={DollarSign}
            />
            <StatCard
              title={text("平均 RPM", "Average RPM")}
              value={formatRate(averageRpm)}
              icon={Activity}
              hint={scopeHint()}
            />
            <StatCard
              title={text("平均 TPM", "Average TPM")}
              value={formatRate(averageTpm)}
              icon={Coins}
              hint={scopeHint()}
            />
          </div>

          <div className="min-w-0 overflow-hidden rounded-[28px] border border-base-300 bg-base-100 p-5 shadow-sm md:p-6">
            <div className="flex flex-wrap items-start justify-between gap-4">
              <div>
                <div className="text-xs uppercase tracking-[0.18em] text-base-content/45">
                  {distributionMetric === "cost"
                    ? text("模型消耗分布", "Model Spend Distribution")
                    : text("模型 Token 分布", "Model Token Distribution")}
                </div>
                <p className="mt-3 text-sm text-base-content/55">
                  {text(
                    "堆叠柱状图按照时间窗口展示各模型消耗，颜色越高越容易看出主力模型。",
                    "Stacked bars break down consumption by model over time, making the dominant models obvious.",
                  )}
                </p>
              </div>
              <div className="rounded-3xl bg-emerald-500/10 px-4 py-3 text-emerald-700">
                <div className="text-xs uppercase tracking-[0.18em] text-emerald-700/70">
                  {distributionMetric === "cost"
                    ? text("窗口总费用", "Window total")
                    : text("窗口总 Token", "Window total")}
                </div>
                <div className="mt-2 text-2xl font-semibold">
                  {distributionMetric === "cost"
                    ? `$${formatUsd(stats?.summary.total_estimated_cost ?? 0)}`
                    : formatTokenCount(totalTokens)}
                </div>
              </div>
            </div>

            {distributionBuckets.length === 0 || modelRankingRows.length === 0 ? (
              <div className="mt-6 rounded-2xl border border-dashed border-base-300 px-4 py-10 text-center text-sm text-base-content/55">
                {text("暂时还没有可展示的 Token 分布数据。", "No token distribution data yet.")}
              </div>
            ) : (
              <div className="mt-6 space-y-5">
                <div className="grid min-w-0 gap-4 xl:grid-cols-[72px_minmax(0,1fr)]">
                  <div className="relative hidden h-72 xl:block">
                    {[1, 0.75, 0.5, 0.25, 0].map((ratio) => (
                      <div
                        key={ratio}
                        className="absolute left-0 right-0 flex -translate-y-1/2 items-center justify-end"
                        style={{ top: `${(1 - ratio) * 100}%` }}
                      >
                        <span className="text-xs font-medium text-base-content/40">
                          {formatDistributionValue(maxDistributionValue * ratio)}
                        </span>
                      </div>
                    ))}
                  </div>

                  <div className="relative min-w-0 overflow-hidden">
                    <div className="pointer-events-none absolute inset-0 flex flex-col justify-between py-3">
                      {[0, 1, 2, 3, 4].map((line) => (
                        <div key={line} className="border-t border-base-300/70" />
                      ))}
                    </div>

                    <div className="relative flex h-72 items-end gap-2 overflow-x-auto px-1 pb-6 pt-3">
                      {distributionBuckets.map((bucket) => {
                        const totalValue =
                          distributionMetric === "cost" ? bucket.total_cost : bucket.total_tokens;
                        const barHeight = totalValue > 0
                          ? Math.max((totalValue / maxDistributionValue) * 100, 4)
                          : 1.5;

                        return (
                          <div
                            key={bucket.timestamp}
                            className="flex h-full min-w-[58px] flex-1 flex-col items-center"
                            title={chartTooltip(bucket)}
                          >
                            <div className="group flex h-full w-full items-end">
                              <div className="flex h-full w-full items-end rounded-[24px] border border-base-300/70 bg-base-200/45 p-[6px] shadow-sm transition duration-150 group-hover:-translate-y-1 group-hover:shadow-md">
                                <div
                                  className="w-full overflow-hidden rounded-[18px] bg-base-100/80"
                                  style={{ height: `${barHeight}%` }}
                                >
                                  <div className="flex h-full w-full flex-col justify-end">
                                    {bucket.segments
                                      .filter((segment) => {
                                        const segmentValue =
                                          distributionMetric === "cost" ? segment.cost : segment.total_tokens;
                                        return segmentValue > 0;
                                      })
                                      .map((segment) => {
                                        const segmentValue =
                                          distributionMetric === "cost" ? segment.cost : segment.total_tokens;
                                        const segmentHeight = totalValue > 0
                                          ? (segmentValue / totalValue) * 100
                                          : 0;
                                        return (
                                          <div
                                            key={`${bucket.timestamp}-${segment.model}`}
                                            className="w-full"
                                            style={{
                                              height: `${segmentHeight}%`,
                                              backgroundColor:
                                                colorByModel.get(segment.model) ?? "#CBD5E1",
                                            }}
                                          />
                                        );
                                      })}
                                  </div>
                                </div>
                              </div>
                            </div>
                            <div className="mt-3 text-[11px] font-medium text-base-content/50">
                              {formatBucketLabel(bucket.timestamp)}
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  </div>
                </div>

                <div className="flex flex-wrap gap-2">
                  {modelRankingRows.map((row, index) => (
                    <div
                      key={row.model}
                      className="flex max-w-full items-center gap-2 rounded-full border border-base-300 bg-base-100 px-3 py-1.5 text-xs shadow-sm"
                    >
                      <span
                        className="h-2.5 w-2.5 shrink-0 rounded-full"
                        style={{
                          backgroundColor:
                            colorByModel.get(row.model) ??
                            DISTRIBUTION_COLORS[index % DISTRIBUTION_COLORS.length],
                        }}
                      />
                      <span className="max-w-[14rem] truncate" title={row.model}>
                        {row.model}
                      </span>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>

          <div className="grid grid-cols-1 gap-4 xl:grid-cols-[1.2fr_0.8fr]">
            <div className="rounded-[28px] border border-base-300 bg-base-100 p-5 shadow-sm md:p-6">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <div className="text-xs uppercase tracking-[0.18em] text-base-content/45">
                    {text("热门模型", "Top Models")}
                  </div>
                  <h2 className="mt-2 text-lg font-semibold">
                    {text("消耗最高的模型排行", "Highest-consumption models")}
                  </h2>
                </div>
                <div className="rounded-full bg-base-200 px-3 py-1 text-xs text-base-content/55">
                  {distributionMetric === "cost"
                    ? text("按费用排序", "Sorted by cost")
                    : text("按 Token 排序", "Sorted by tokens")}
                </div>
              </div>

              <div className="mt-5 space-y-3">
                {modelRankingRows.length === 0 ? (
                  <div className="rounded-2xl border border-dashed border-base-300 px-4 py-8 text-center text-sm text-base-content/55">
                    {text("暂时还没有模型统计。", "No model stats yet.")}
                  </div>
                ) : (
                  modelRankingRows.map((row, index) => {
                    const share = (metricValue(row, distributionMetric) / maxModelMetricValue) * 100;
                    const rowColor =
                      colorByModel.get(row.model) ??
                      DISTRIBUTION_COLORS[index % DISTRIBUTION_COLORS.length];
                    return (
                      <div
                        key={row.model}
                        className="rounded-2xl border border-base-300/80 bg-base-100 px-4 py-4 shadow-sm"
                      >
                        <div className="flex items-start justify-between gap-4">
                          <div className="min-w-0 flex-1">
                            <div className="flex items-center gap-3">
                              <span
                                className="h-2.5 w-2.5 shrink-0 rounded-full"
                                style={{ backgroundColor: rowColor }}
                              />
                              <div className="truncate font-medium" title={row.model}>
                                {row.model}
                              </div>
                            </div>
                            <div className="mt-2 flex flex-wrap gap-3 text-xs text-base-content/55">
                              <span>{text("请求", "Requests")}: {row.total_requests}</span>
                              <span>{text("输入", "Input")}: {formatTokenCount(row.total_input_tokens)}</span>
                              <span>{text("输出", "Output")}: {formatTokenCount(row.total_output_tokens)}</span>
                            </div>
                          </div>
                          <div className="text-right">
                            <div className="text-sm font-semibold">
                              {distributionMetric === "cost"
                                ? `$${formatUsd(row.total_cost)}`
                                : formatTokenCount(row.total_input_tokens + row.total_output_tokens)}
                            </div>
                            <div className="mt-1 text-xs text-base-content/45">
                              {text("费用", "Cost")}: ${formatUsd(row.total_cost)}
                            </div>
                          </div>
                        </div>
                        <div className="mt-4 h-2 rounded-full bg-base-200">
                          <div
                            className="h-full rounded-full"
                            style={{
                              width: `${Math.max(share, 4)}%`,
                              backgroundColor: rowColor,
                            }}
                          />
                        </div>
                      </div>
                    );
                  })
                )}
              </div>
            </div>

            <div className="rounded-[28px] border border-base-300 bg-base-100 p-5 shadow-sm md:p-6">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <div className="text-xs uppercase tracking-[0.18em] text-base-content/45">
                    {text("站点消耗", "Site Usage")}
                  </div>
                  <h2 className="mt-2 text-lg font-semibold">
                    {text("查看哪个站点在承担主要流量", "See which sites carry the most traffic")}
                  </h2>
                </div>
                <Server size={18} className="text-base-content/35" />
              </div>

              <div className="mt-5 space-y-3">
                {accountRows.length === 0 ? (
                  <div className="rounded-2xl border border-dashed border-base-300 px-4 py-8 text-center text-sm text-base-content/55">
                    {text("暂时还没有站点统计。", "No site stats yet.")}
                  </div>
                ) : (
                  accountRows.slice(0, 8).map(([accountId, row]) => {
                    const account = config.proxy_accounts.find((item) => item.id === accountId);
                    const label = account
                      ? `${account.site_name}${account.account_info.username ? ` (${account.account_info.username})` : ""}`
                      : accountId;
                    const width = (row.total_estimated_cost / maxAccountCost) * 100;

                    return (
                      <div
                        key={accountId}
                        className="rounded-2xl border border-base-300/80 bg-base-100 px-4 py-4 shadow-sm"
                      >
                        <div className="flex items-start justify-between gap-3">
                          <div className="min-w-0 flex-1">
                            <div className="truncate font-medium" title={label}>
                              {label}
                            </div>
                            <div className="mt-2 flex flex-wrap gap-3 text-xs text-base-content/55">
                              <span>{text("请求", "Requests")}: {row.total_requests}</span>
                              <span>{text("输入", "Input")}: {formatTokenCount(row.total_input_tokens)}</span>
                              <span>{text("输出", "Output")}: {formatTokenCount(row.total_output_tokens)}</span>
                            </div>
                          </div>
                          <div className="text-right text-sm font-semibold">
                            ${formatUsd(row.total_estimated_cost)}
                          </div>
                        </div>
                        <div className="mt-4 h-2 rounded-full bg-base-200">
                          <div
                            className="h-full rounded-full bg-primary"
                            style={{ width: `${Math.max(width, 4)}%` }}
                          />
                        </div>
                      </div>
                    );
                  })
                )}
              </div>
            </div>
          </div>

          <div className="rounded-[28px] border border-base-300 bg-base-100 p-5 shadow-sm md:p-6">
            <div className="flex items-center justify-between gap-3">
              <div>
                <div className="text-xs uppercase tracking-[0.18em] text-base-content/45">
                  {text("时间趋势", "Timeline")}
                </div>
                <h2 className="mt-2 text-lg font-semibold">
                  {text("请求、Token 与费用概览", "Requests, tokens, and cost overview")}
                </h2>
              </div>
              <BarChart3 size={18} className="text-base-content/35" />
            </div>

            {stats?.timeline.length ? (
              <div className="mt-5 grid grid-cols-1 gap-3 md:grid-cols-3">
                {stats.timeline.slice(-3).map((bucket) => (
                  <div
                    key={bucket.timestamp}
                    className="rounded-2xl border border-base-300/80 bg-base-100 px-4 py-4 shadow-sm"
                  >
                    <div className="text-xs uppercase tracking-[0.14em] text-base-content/45">
                      {formatBucketLabel(bucket.timestamp)}
                    </div>
                    <div className="mt-3 space-y-2 text-sm">
                      <div className="flex items-center justify-between">
                        <span className="text-base-content/60">{text("请求", "Requests")}</span>
                        <span className="font-medium">{bucket.total_requests}</span>
                      </div>
                      <div className="flex items-center justify-between">
                        <span className="text-base-content/60">{text("总 Token", "Total Tokens")}</span>
                        <span className="font-medium">{formatTokenCount(bucket.total_tokens)}</span>
                      </div>
                      <div className="flex items-center justify-between">
                        <span className="text-base-content/60">{text("费用", "Cost")}</span>
                        <span className="font-medium">${formatUsd(bucket.total_cost)}</span>
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            ) : (
              <div className="mt-5 rounded-2xl border border-dashed border-base-300 px-4 py-8 text-center text-sm text-base-content/55">
                {text("暂时还没有时间趋势数据。", "No timeline data yet.")}
              </div>
            )}
          </div>
        </>
      )}
    </div>
  );
}
