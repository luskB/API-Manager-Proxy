import { Fragment, useEffect, useState } from "react";
import { request } from "../utils/request";
import {
  RefreshCw,
  Timer,
  CheckCircle,
  XCircle,
  ChevronDown,
  ChevronRight,
  Copy,
  Play,
  RotateCcw,
} from "lucide-react";
import { cn } from "../utils/cn";
import { useConfig } from "../hooks/useConfig";
import { useLocale } from "../hooks/useLocale";

let monitorAutoRefreshEnabled = true;

interface SiteBillingSnapshot {
  created_at?: number;
  model_name?: string;
  token_name?: string;
  quota?: number;
  prompt_tokens?: number;
  completion_tokens?: number;
  content?: string;
  other?: unknown;
  raw: unknown;
}

interface ProxyRequestLog {
  id: string;
  timestamp: number;
  method: string;
  url: string;
  status: number;
  duration_ms: number;
  model?: string;
  account_id?: string;
  upstream_url?: string;
  client_ip?: string;
  input_tokens?: number;
  output_tokens?: number;
  estimated_cost?: number;
  error?: string;
  request_body?: string;
  original_request_body?: string;
  response_body?: string;
  cost_source?: string;
  site_cost_text?: string;
  site_billing?: SiteBillingSnapshot;
  billing_synced_at?: number;
}

interface LogsResponse {
  total: number;
  logs: ProxyRequestLog[];
}

interface SyncLogCostsResponse {
  matched: number;
  unmatched: number;
  failed_accounts: number;
  synced_log_ids: string[];
}

const DEFAULT_WINDOW_MINUTES = 60;

export default function MonitorPage() {
  const [logs, setLogs] = useState<ProxyRequestLog[]>([]);
  const [total, setTotal] = useState(0);
  const [autoRefresh, setAutoRefresh] = useState(() => monitorAutoRefreshEnabled);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [replayResult, setReplayResult] = useState<{ status: number; body: string } | null>(null);
  const [replaying, setReplaying] = useState(false);
  const [syncingCosts, setSyncingCosts] = useState(false);
  const [windowMinutes, setWindowMinutes] = useState(DEFAULT_WINDOW_MINUTES);
  const [syncMessage, setSyncMessage] = useState("");
  const { config } = useConfig();
  const [logError, setLogError] = useState("");
  const { t } = useLocale();

  useEffect(() => {
    monitorAutoRefreshEnabled = autoRefresh;
  }, [autoRefresh]);

  useEffect(() => {
    loadLogs(windowMinutes);
  }, [windowMinutes]);

  useEffect(() => {
    if (!autoRefresh) return;
    const timer = setInterval(() => {
      void loadLogs(windowMinutes);
    }, 3000);
    return () => clearInterval(timer);
  }, [autoRefresh, windowMinutes]);

  async function loadLogs(minutes = windowMinutes) {
    try {
      const data = await request<LogsResponse>("get_logs", { minutes });
      setLogs(data.logs || []);
      setTotal(data.total || 0);
      setLogError("");
    } catch (e) {
      setLogError(String(e));
    }
  }

  async function syncCosts() {
    setSyncingCosts(true);
    setSyncMessage("");
    setLogError("");
    try {
      const result = await request<SyncLogCostsResponse>("sync_log_costs", {
        minutes: windowMinutes,
        log_ids: logs.map((log) => log.id),
      });
      const message = `${t("monitor.syncDone")}: ${result.matched} ${t("common.success").toLowerCase()}, ${result.unmatched} ${t("monitor.unmatched").toLowerCase()}`;
      setSyncMessage(message);
      await loadLogs(windowMinutes);
    } catch (e) {
      setLogError(String(e));
    } finally {
      setSyncingCosts(false);
    }
  }

  function formatTime(ts: number): string {
    return new Date(ts * 1000).toLocaleTimeString();
  }

  function formatDateTime(ts: number): string {
    return new Date(ts * 1000).toLocaleString();
  }

  function copyCurl(log: ProxyRequestLog) {
    const port = config?.proxy.port ?? 3000;
    const parts = [`curl -X ${log.method} 'http://localhost:${port}${log.url}'`];
    parts.push("-H 'Content-Type: application/json'");
    const replayBody = log.original_request_body ?? log.request_body;
    if (replayBody) {
      const escaped = replayBody.replace(/'/g, "'\\''");
      parts.push(`-d '${escaped}'`);
    }
    navigator.clipboard.writeText(parts.join(" \\\n  "));
  }

  async function handleReplay(logId: string) {
    setReplaying(true);
    setReplayResult(null);
    try {
      const result = await request<{ status: number; body: string }>("replay_request", {
        log_id: logId,
        logId,
      });
      setReplayResult(result);
    } catch (e) {
      setReplayResult({ status: 0, body: String(e) });
    } finally {
      setReplaying(false);
    }
  }

  function formatJson(raw: unknown): string {
    if (raw == null) return "";
    if (typeof raw === "string") {
      try {
        return JSON.stringify(JSON.parse(raw), null, 2);
      } catch {
        return raw;
      }
    }
    try {
      return JSON.stringify(raw, null, 2);
    } catch {
      return String(raw);
    }
  }

  function numericCostValue(log: ProxyRequestLog): number | null {
    if (log.cost_source === "site_log" && log.site_cost_text) {
      const raw = Number(log.site_cost_text);
      if (Number.isFinite(raw)) {
        return raw;
      }
    }
    if (typeof log.estimated_cost === "number") {
      return log.estimated_cost;
    }
    return null;
  }

  function formatSiteNumber(value: number): string {
    if (Math.abs(value % 1) < Number.EPSILON) {
      return value.toFixed(0);
    }
    return value.toFixed(4).replace(/0+$/, "").replace(/\.$/, "");
  }

  function costDisplay(log: ProxyRequestLog): string {
    if (log.cost_source === "site_log" && log.site_cost_text) {
      return log.site_cost_text;
    }
    if (log.cost_source === "site_unmatched") {
      return t("monitor.notSynced");
    }
    if (log.estimated_cost != null) {
      return `$${log.estimated_cost.toFixed(4)}`;
    }
    return "-";
  }

  const successCount = logs.filter((l) => l.status >= 200 && l.status < 300).length;
  const errorCount = logs.filter((l) => l.status >= 400).length;
  const totalCost = logs.reduce((sum, log) => sum + (numericCostValue(log) ?? 0), 0);
  const hasSyncedSiteCost = logs.some((log) => log.cost_source === "site_log");

  return (
    <div className="space-y-6">
      <div className="flex flex-col gap-3 xl:flex-row xl:items-center xl:justify-between">
        <div>
          <h1 className="text-2xl font-bold">{t("monitor.title")}</h1>
          <p className="text-base-content/60 mt-1">
            {t("monitor.subtitle")} ({total} {t("common.total").toLowerCase()})
          </p>
        </div>
        <div className="flex w-full flex-nowrap items-center gap-2 overflow-x-auto xl:flex-1 xl:pl-6">
          <label className="input input-sm input-bordered flex shrink-0 items-center gap-2">
            <span className="text-xs text-base-content/60">{t("monitor.windowMinutes")}</span>
            <input
              type="number"
              min={1}
              step={1}
              className="w-20"
              value={windowMinutes}
              onChange={(e) => {
                const next = Number(e.target.value);
                setWindowMinutes(Number.isFinite(next) && next > 0 ? next : DEFAULT_WINDOW_MINUTES);
              }}
            />
          </label>
          <button className="btn btn-ghost btn-sm shrink-0 gap-2" onClick={() => loadLogs(windowMinutes)}>
            <RefreshCw size={14} />
            {t("common.refresh")}
          </button>
          <button
            className={cn(
              "btn btn-sm shrink-0 gap-2",
              autoRefresh ? "btn-primary" : "btn-ghost"
            )}
            onClick={() => setAutoRefresh(!autoRefresh)}
          >
            <Timer size={14} />
            {autoRefresh ? t("monitor.autoOn") : t("monitor.autoOff")}
          </button>
          <button
            className="btn btn-ghost btn-sm shrink-0 gap-2"
            onClick={() => void syncCosts()}
            disabled={syncingCosts || logs.length === 0}
          >
            <RotateCcw size={14} className={cn(syncingCosts && "animate-spin")} />
            {syncingCosts ? t("monitor.syncing") : t("monitor.syncCosts")}
          </button>
        </div>
      </div>

      {syncMessage && (
        <div role="alert" className="alert alert-success">
          <span>{syncMessage}</span>
        </div>
      )}

      {logError && (
        <div role="alert" className="alert alert-error">
          <span>{logError}</span>
        </div>
      )}

      <div className="stats stats-horizontal bg-base-100 border border-base-300 w-full">
        <div className="stat">
          <div className="stat-title">{t("common.total")}</div>
          <div className="stat-value text-lg">{total}</div>
        </div>
        <div className="stat">
          <div className="stat-figure text-success">
            <CheckCircle size={20} />
          </div>
          <div className="stat-title">{t("common.success")}</div>
          <div className="stat-value text-lg text-success">{successCount}</div>
        </div>
        <div className="stat">
          <div className="stat-figure text-error">
            <XCircle size={20} />
          </div>
          <div className="stat-title">{t("common.error")}</div>
          <div className="stat-value text-lg text-error">{errorCount}</div>
        </div>
        {totalCost > 0 && (
          <div className="stat">
            <div className="stat-title">{t("monitor.totalCost")}</div>
            <div className="stat-value text-lg">
              {hasSyncedSiteCost ? formatSiteNumber(totalCost) : `$${totalCost.toFixed(4)}`}
            </div>
          </div>
        )}
      </div>

      {logs.length === 0 ? (
        <div className="card bg-base-100 border border-base-300">
          <div className="card-body items-center text-center py-12">
            <p className="text-base-content/60">{t("monitor.noLogs")}</p>
            <p className="text-sm text-base-content/40 mt-1">
              {t("monitor.noLogsHint")}
            </p>
          </div>
        </div>
      ) : (
        <div className="card bg-base-100 border border-base-300">
          <div className="overflow-x-auto">
            <table className="table table-sm">
              <thead>
                <tr>
                  <th></th>
                  <th>{t("monitor.time")}</th>
                  <th>{t("monitor.method")}</th>
                  <th>{t("monitor.path")}</th>
                  <th>{t("monitor.upstream")}</th>
                  <th>{t("monitor.model")}</th>
                  <th>{t("monitor.status")}</th>
                  <th>{t("monitor.duration")}</th>
                  <th>{t("monitor.tokens")}</th>
                  <th>{t("monitor.cost")}</th>
                  <th>{t("monitor.actions")}</th>
                </tr>
              </thead>
              <tbody>
                {logs.map((log) => (
                  <Fragment key={log.id}>
                    <tr className="hover cursor-pointer" onClick={() => setExpandedId(expandedId === log.id ? null : log.id)}>
                      <td className="w-6">
                        {expandedId === log.id ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                      </td>
                      <td className="font-mono text-xs">
                        {formatTime(log.timestamp)}
                      </td>
                      <td>
                        <span className="badge badge-outline badge-xs">
                          {log.method}
                        </span>
                      </td>
                      <td className="text-xs max-w-[200px] truncate">{log.url}</td>
                      <td className="text-xs max-w-[200px] truncate" title={log.upstream_url}>{log.upstream_url || "-"}</td>
                      <td className="text-xs">{log.model || "-"}</td>
                      <td>
                        <span
                          className={cn(
                            "badge badge-sm",
                            log.status >= 200 && log.status < 300
                              ? "badge-success"
                              : log.status >= 400 && log.status < 500
                                ? "badge-warning"
                                : "badge-error"
                          )}
                        >
                          {log.status}
                        </span>
                      </td>
                      <td className="text-xs font-mono">{log.duration_ms}ms</td>
                      <td className="text-xs">
                        {log.input_tokens != null
                          ? `${log.input_tokens}/${log.output_tokens}`
                          : "-"}
                      </td>
                      <td className="text-xs font-mono">
                        {costDisplay(log)}
                      </td>
                      <td onClick={(e) => e.stopPropagation()}>
                        <div className="flex gap-1">
                          <button
                            className="btn btn-ghost btn-xs tooltip"
                            data-tip={t("monitor.copyAsCurl")}
                            onClick={() => copyCurl(log)}
                          >
                            <Copy size={12} />
                          </button>
                          {log.request_body && (
                            <button
                              className="btn btn-ghost btn-xs tooltip"
                              data-tip={t("monitor.replay")}
                              disabled={replaying}
                              onClick={() => handleReplay(log.id)}
                            >
                              <Play size={12} />
                            </button>
                          )}
                        </div>
                      </td>
                    </tr>
                    {expandedId === log.id && (
                      <tr key={`${log.id}-detail`}>
                        <td colSpan={11} className="bg-base-200/50 p-0">
                          <div className="p-4 space-y-3">
                            {log.error && (
                              <div className="text-error text-xs">{t("common.error")}: {log.error}</div>
                            )}
                            {log.client_ip && (
                              <div className="text-xs text-base-content/60">{t("monitor.clientIp")}: {log.client_ip}</div>
                            )}
                            <div className="grid grid-cols-1 lg:grid-cols-2 gap-3">
                              <div>
                                <div className="text-xs font-semibold mb-1">{t("monitor.requestBody")}</div>
                                <pre className="bg-base-300 rounded p-2 text-xs overflow-auto max-h-64 whitespace-pre-wrap break-all">
                                  {log.request_body ? formatJson(log.request_body) : t("monitor.notCaptured")}
                                </pre>
                              </div>
                              <div>
                                <div className="text-xs font-semibold mb-1">{t("monitor.responseBody")}</div>
                                <pre className="bg-base-300 rounded p-2 text-xs overflow-auto max-h-64 whitespace-pre-wrap break-all">
                                  {log.response_body ? formatJson(log.response_body) : t("monitor.notCaptured")}
                                </pre>
                              </div>
                            </div>

                            <div className="rounded-lg border border-base-300 bg-base-100 p-3 space-y-2">
                              <div className="text-xs font-semibold">{t("monitor.siteBilling")}</div>
                              {log.cost_source === "site_log" && log.site_billing ? (
                                <div className="space-y-2 text-xs">
                                  <div className="text-base-content/70">
                                    {t("monitor.syncedAt")}: {log.billing_synced_at ? formatDateTime(log.billing_synced_at) : "-"}
                                  </div>
                                  <div className="text-base-content/70">
                                    {t("monitor.siteCharge")}: {log.site_cost_text || "-"}
                                  </div>
                                  {log.site_billing.content && (
                                    <div className="text-base-content/80 whitespace-pre-wrap break-all">
                                      {log.site_billing.content}
                                    </div>
                                  )}
                                  <pre className="bg-base-300 rounded p-2 text-xs overflow-auto max-h-64 whitespace-pre-wrap break-all">
                                    {formatJson(log.site_billing.raw)}
                                  </pre>
                                </div>
                              ) : log.cost_source === "site_unmatched" ? (
                                <div className="text-xs text-base-content/60">{t("monitor.notSyncedHint")}</div>
                              ) : (
                                <div className="text-xs text-base-content/60">{t("monitor.siteBillingHint")}</div>
                              )}
                            </div>

                            {replayResult && expandedId === log.id && (
                              <div>
                                <div className="text-xs font-semibold mb-1">{t("monitor.replayResult")} (status: {replayResult.status})</div>
                                <pre className="bg-base-300 rounded p-2 text-xs overflow-auto max-h-48 whitespace-pre-wrap break-all">
                                  {formatJson(replayResult.body)}
                                </pre>
                              </div>
                            )}
                          </div>
                        </td>
                      </tr>
                    )}
                  </Fragment>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}
    </div>
  );
}
