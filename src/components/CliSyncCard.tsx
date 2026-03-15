import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  Bot,
  CodeXml,
  Cpu,
  Globe,
  Search,
  Sparkles,
  Terminal,
  TriangleAlert,
  X,
} from "lucide-react";
import CliAppCard from "./CliAppCard";
import ConfigEditorModal from "./ConfigEditorModal";
import { request } from "../utils/request";
import { useLocale } from "../hooks/useLocale";
import { cn } from "../utils/cn";

type CliAppType = "Claude" | "Codex" | "Gemini" | "OpenCode" | "Droid";

interface CliStatus {
  installed: boolean;
  version: string | null;
  is_synced: boolean;
  has_backup: boolean;
  current_base_url: string | null;
  files: string[];
}

interface ClaudeModelConfig {
  model: string | null;
  primaryModel: string | null;
  haikuModel: string | null;
  opusModel: string | null;
  sonnetModel: string | null;
  reasoningModel: string | null;
}

interface ProxyModelCatalogRow {
  account_id: string;
  account_selector: string;
  site_name: string;
  models: string[];
}

interface CliSyncRoute {
  model_pattern: string;
  account_ids: string[];
  priority: number;
  managed_by?: string;
}

interface CliSyncCardProps {
  proxyUrl: string;
  apiKey: string;
  proxyPort: number;
  modelCatalog: ProxyModelCatalogRow[];
  persistedCliRoutes?: CliSyncRoute[];
  onPersistCliRoutes?: (routes: CliSyncRoute[]) => Promise<void>;
}

interface CliProbeResult {
  cli_name: string;
  config_found: boolean;
  config_valid: boolean;
  proxy_reachable: boolean;
  auth_ok: boolean;
  model_available: boolean;
  response_valid: boolean;
  error: string | null;
  latency_ms: number;
}

interface EditorState {
  app: CliAppType;
  fileName: string;
  allFiles: string[];
  content: string;
  isGenerated: boolean;
  isValid: boolean;
}

interface ConfirmDialogState {
  title: string;
  message: string;
  confirmLabel: string;
  tone: "primary" | "danger";
  onConfirm: () => void;
}

interface CliModelOption {
  key: string;
  accountId: string;
  accountSelector: string;
  siteName: string;
  model: string;
  displayLabel: string;
  searchText: string;
}

const CLI_APPS: CliAppType[] = ["Claude", "Codex", "Gemini", "OpenCode", "Droid"];

const iconMap: Record<CliAppType, ReactNode> = {
  Claude: <CodeXml size={20} className="text-purple-500" />,
  Codex: <Cpu size={20} className="text-blue-500" />,
  Gemini: <Globe size={20} className="text-green-500" />,
  OpenCode: <CodeXml size={20} className="text-cyan-500" />,
  Droid: <Bot size={20} className="text-orange-500" />,
};

const nameMap: Record<CliAppType, string> = {
  Claude: "Claude Code",
  Codex: "Codex CLI",
  Gemini: "Gemini CLI",
  OpenCode: "OpenCode",
  Droid: "Droid",
};

function buildClaudeModelConfig(models: string[]): ClaudeModelConfig {
  const claude = models.filter(
    (model) => model.includes("claude") || model.includes("anthropic"),
  );
  if (claude.length === 0) {
    return {
      model: null,
      primaryModel: null,
      haikuModel: null,
      opusModel: null,
      sonnetModel: null,
      reasoningModel: null,
    };
  }

  const findBest = (keywords: string[], fallback?: string): string | null => {
    for (const keyword of keywords) {
      const match = claude.find((model) => model.includes(keyword));
      if (match) return match;
    }
    return fallback || null;
  };

  const opus = findBest(["opus-4-6", "opus-4", "opus"]);
  const sonnet = findBest([
    "sonnet-4",
    "claude-4-sonnet",
    "sonnet-3-7",
    "sonnet-3-5",
    "sonnet-3.5",
    "sonnet",
  ]);
  const haiku = findBest(["haiku-4-5", "haiku-4", "haiku-3.5", "haiku"]);
  const primary = opus || sonnet || haiku || claude[0];
  const reasoning = opus || primary;
  const modelAlias = opus ? "opus" : sonnet ? "sonnet" : "haiku";

  return {
    model: modelAlias,
    primaryModel: primary,
    haikuModel: haiku || primary,
    opusModel: opus || primary,
    sonnetModel: sonnet || primary,
    reasoningModel: reasoning,
  };
}

function dedupe(values: string[]): string[] {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const value of values) {
    if (!value || seen.has(value)) continue;
    seen.add(value);
    result.push(value);
  }
  return result;
}

function dedupeModelOptionsByBareModel(options: CliModelOption[]): CliModelOption[] {
  const seen = new Set<string>();
  const result: CliModelOption[] = [];
  for (const option of options) {
    if (!option.model || seen.has(option.model)) continue;
    seen.add(option.model);
    result.push(option);
  }
  return result;
}

function isValidJson(str: string): boolean {
  try {
    JSON.parse(str);
    return true;
  } catch {
    return false;
  }
}

function initRecord<T>(val: T): Record<CliAppType, T> {
  return { Claude: val, Codex: val, Gemini: val, OpenCode: val, Droid: val };
}

function buildCliRoutes(options: CliModelOption[]): CliSyncRoute[] {
  return options.map((option, index) => ({
    model_pattern: option.model,
    account_ids: [option.accountId],
    priority: index,
    managed_by: "cli_sync",
  }));
}

export default function CliSyncCard({
  proxyUrl,
  apiKey,
  proxyPort,
  modelCatalog,
  persistedCliRoutes = [],
  onPersistCliRoutes,
}: CliSyncCardProps) {
  const [statuses, setStatuses] = useState<Record<CliAppType, CliStatus | null>>(
    initRecord(null),
  );
  const [loading, setLoading] = useState<Record<CliAppType, boolean>>(
    initRecord(false),
  );
  const [syncing, setSyncing] = useState<Record<CliAppType, boolean>>(
    initRecord(false),
  );
  const [testing, setTesting] = useState<Record<CliAppType, boolean>>(
    initRecord(false),
  );
  const [probeResults, setProbeResults] = useState<
    Record<CliAppType, CliProbeResult | null>
  >(initRecord(null));
  const [selectedModelKeys, setSelectedModelKeys] = useState<string[]>([]);
  const [modelFilter, setModelFilter] = useState("");
  const [editorState, setEditorState] = useState<EditorState | null>(null);
  const [confirmDialog, setConfirmDialog] = useState<ConfirmDialogState | null>(null);
  const lastHydratedSelectionRef = useRef("");
  const { t, locale } = useLocale();

  const text = (zh: string, en: string) => (locale === "zh" ? zh : en);

  const getFormattedProxyUrl = useCallback(
    (app: CliAppType) => {
      if (!proxyUrl) return "";
      const base = proxyUrl.trimEnd().replace(/\/+$/, "");
      if (app === "Codex" || app === "OpenCode") {
        return base.endsWith("/v1") ? base : `${base}/v1`;
      }
      return base.replace(/\/v1$/, "");
    },
    [proxyUrl],
  );

  const modelOptions = useMemo(() => {
    const options: CliModelOption[] = [];
    for (const row of modelCatalog) {
      for (const model of row.models ?? []) {
        if (!model) continue;
        const siteName = row.site_name?.trim() || row.account_selector?.trim() || row.account_id;
        const displayLabel = `${siteName}::${model}`;
        options.push({
          key: `${row.account_id}::${model}`,
          accountId: row.account_id,
          accountSelector: row.account_selector,
          siteName,
          model,
          displayLabel,
          searchText: `${displayLabel} ${row.account_selector} ${row.account_id}`.toLowerCase(),
        });
      }
    }
    return options.sort((left, right) =>
      left.displayLabel.localeCompare(right.displayLabel),
    );
  }, [modelCatalog]);

  const optionMap = useMemo(
    () => new Map(modelOptions.map((option) => [option.key, option])),
    [modelOptions],
  );

  const visibleModelOptions = useMemo(() => {
    const keyword = modelFilter.trim().toLowerCase();
    if (!keyword) return modelOptions;
    return modelOptions.filter((option) => option.searchText.includes(keyword));
  }, [modelFilter, modelOptions]);

  const persistedSelectedKeys = useMemo(
    () =>
      dedupe(
        persistedCliRoutes.flatMap((route) => {
          const accountId = route.account_ids?.[0];
          const model = route.model_pattern?.trim();
          if (!accountId || !model) return [];
          return [`${accountId}::${model}`];
        }),
      ),
    [persistedCliRoutes],
  );

  const persistedSelectionSignature = useMemo(
    () => persistedSelectedKeys.join("\u0001"),
    [persistedSelectedKeys],
  );

  const effectiveSelectedOptions = useMemo(() => {
    const ordered = selectedModelKeys
      .map((key) => optionMap.get(key))
      .filter((option): option is CliModelOption => Boolean(option));
    return dedupeModelOptionsByBareModel(ordered);
  }, [optionMap, selectedModelKeys]);

  const selectedBindingOptions = useMemo(
    () =>
      selectedModelKeys
        .map((key) => optionMap.get(key))
        .filter((option): option is CliModelOption => Boolean(option)),
    [optionMap, selectedModelKeys],
  );

  const effectiveModels = useMemo(
    () => effectiveSelectedOptions.map((option) => option.model),
    [effectiveSelectedOptions],
  );

  const defaultModel = effectiveModels[0] ?? null;
  const allModelsSelected =
    modelOptions.length > 0 && selectedModelKeys.length === modelOptions.length;
  const useAllDiscoveredModels = allModelsSelected;
  const duplicateSiteBindings: string[] = [];

  const getClaudeModels = useCallback(
    (models: string[]): ClaudeModelConfig | null => {
      if (models.length === 0) return null;
      return buildClaudeModelConfig(models);
    },
    [],
  );

  const buildConfigPayload = useCallback(
    (app: CliAppType, fileName?: string) => ({
      appType: app,
      proxyUrl: getFormattedProxyUrl(app),
      apiKey,
      model: defaultModel,
      models: effectiveModels,
      claudeModels: app === "Claude" ? getClaudeModels(effectiveModels) : null,
      ...(fileName ? { fileName } : {}),
    }),
    [apiKey, defaultModel, effectiveModels, getClaudeModels, getFormattedProxyUrl],
  );

  const checkStatus = useCallback(
    async (app: CliAppType) => {
      setLoading((prev) => ({ ...prev, [app]: true }));
      try {
        const status = await request<CliStatus>("get_cli_sync_status", {
          appType: app,
          proxyUrl: getFormattedProxyUrl(app),
        });
        setStatuses((prev) => ({ ...prev, [app]: status }));
      } catch (err) {
        console.error(`Failed to check ${app} status:`, err);
      } finally {
        setLoading((prev) => ({ ...prev, [app]: false }));
      }
    },
    [getFormattedProxyUrl],
  );

  useEffect(() => {
    if (persistedSelectionSignature === lastHydratedSelectionRef.current) {
      return;
    }

    const hydrated =
      optionMap.size > 0
        ? persistedSelectedKeys.filter((key) => optionMap.has(key))
        : persistedSelectedKeys;
    setSelectedModelKeys(hydrated);
    lastHydratedSelectionRef.current = persistedSelectionSignature;
  }, [optionMap, persistedSelectedKeys, persistedSelectionSignature]);

  useEffect(() => {
    if (optionMap.size === 0) return;
    setSelectedModelKeys((prev) => prev.filter((key) => optionMap.has(key)));
  }, [optionMap]);

  useEffect(() => {
    CLI_APPS.forEach(checkStatus);
  }, [checkStatus]);

  const toggleSelectedModel = (optionKey: string) => {
    setSelectedModelKeys((prev) =>
      prev.includes(optionKey)
        ? prev.filter((item) => item !== optionKey)
        : [...prev, optionKey],
    );
  };

  const selectVisibleModels = () => {
    setSelectedModelKeys((prev) =>
      dedupe([...prev, ...visibleModelOptions.map((option) => option.key)]),
    );
  };

  const syncAllModels = () => {
    if (
      !window.confirm(
        text(
          "确认改为同步全部已发现模型吗？这会清空当前手动选择。",
          "Sync all discovered models? This will clear the current manual selection.",
        ),
      )
    ) {
      return;
    }
    setSelectedModelKeys(modelOptions.map((option) => option.key));
  };

  const clearSelectedModels = () => {
    if (
      !window.confirm(
        text(
          "确认移除当前已选模型吗？移除后需要重新从列表中勾选要同步的模型。",
          "Remove all currently selected models? You can reselect them from the list later.",
        ),
      )
    ) {
      return;
    }
    setSelectedModelKeys([]);
  };

  void syncAllModels;
  void clearSelectedModels;

  const requestSyncAllModels = () => {
    setConfirmDialog({
      title: text("同步全部模型", "Sync all models"),
      message: text(
        "这会把当前已发现的全部站点模型加入同步列表。",
        "This will add every discovered site-model binding to the sync list.",
      ),
      confirmLabel: text("同步全部", "Sync all"),
      tone: "primary",
      onConfirm: () => {
        setSelectedModelKeys(modelOptions.map((option) => option.key));
        setConfirmDialog(null);
      },
    });
  };

  const requestClearSelectedModels = () => {
    setConfirmDialog({
      title: text("移除全部已选模型", "Remove all selected models"),
      message: text(
        "这会清空当前模型同步列表，之后可以再按需要重新添加。",
        "This clears the current sync list. You can add models again later.",
      ),
      confirmLabel: text("全部移除", "Remove all"),
      tone: "danger",
      onConfirm: () => {
        setSelectedModelKeys([]);
        setConfirmDialog(null);
      },
    });
  };

  const handleSync = async (app: CliAppType) => {
    if (!proxyUrl || !apiKey) return;
    const status = statuses[app];
    if (!status) return;
    const firstFile = status.files[0];
    try {
      const content = await request<string>(
        "generate_cli_config",
        buildConfigPayload(app, firstFile),
      );
      const isJson = firstFile.endsWith(".json");
      setEditorState({
        app,
        fileName: firstFile,
        allFiles: status.files,
        content,
        isGenerated: true,
        isValid: isJson ? isValidJson(content) : true,
      });
    } catch (err) {
      console.error("Failed to generate config:", err);
    }
  };

  const handleSwitchFile = async (fileName: string) => {
    if (!editorState) return;
    const { app } = editorState;
    try {
      const content = await request<string>(
        "generate_cli_config",
        buildConfigPayload(app, fileName),
      );
      const isJson = fileName.endsWith(".json");
      setEditorState({
        ...editorState,
        fileName,
        content,
        isGenerated: true,
        isValid: isJson ? isValidJson(content) : true,
      });
    } catch (err) {
      console.error("Failed to generate config:", err);
    }
  };

  const handleApply = async () => {
    if (!editorState || !editorState.isValid) return;
    const { app, fileName, content } = editorState;
    setSyncing((prev) => ({ ...prev, [app]: true }));
    try {
      if (editorState.isGenerated && onPersistCliRoutes) {
        await onPersistCliRoutes(buildCliRoutes(effectiveSelectedOptions));
      }

      await request("write_cli_config", { appType: app, fileName, content });
      for (const file of editorState.allFiles) {
        if (file === fileName) continue;
        try {
          const autoContent = await request<string>(
            "generate_cli_config",
            buildConfigPayload(app, file),
          );
          await request("write_cli_config", {
            appType: app,
            fileName: file,
            content: autoContent,
          });
        } catch {
          // Non-primary files are best-effort.
        }
      }
      setEditorState(null);
      await checkStatus(app);
    } catch (err) {
      console.error("Failed to write config:", err);
    } finally {
      setSyncing((prev) => ({ ...prev, [app]: false }));
    }
  };

  const handleRestore = async (app: CliAppType) => {
    setSyncing((prev) => ({ ...prev, [app]: true }));
    try {
      await request("execute_cli_restore", { appType: app });
      await checkStatus(app);
    } catch (err) {
      console.error("Restore failed:", err);
    } finally {
      setSyncing((prev) => ({ ...prev, [app]: false }));
    }
  };

  const handleTest = async (app: CliAppType) => {
    setTesting((prev) => ({ ...prev, [app]: true }));
    setProbeResults((prev) => ({ ...prev, [app]: null }));
    try {
      const result = await request<CliProbeResult>("probe_cli_compatibility", {
        appType: app,
        proxyPort,
        apiKey,
      });
      setProbeResults((prev) => ({ ...prev, [app]: result }));
    } catch (err) {
      console.error(`Probe ${app} failed:`, err);
      setProbeResults((prev) => ({
        ...prev,
        [app]: {
          cli_name: app,
          config_found: false,
          config_valid: false,
          proxy_reachable: false,
          auth_ok: false,
          model_available: false,
          response_valid: false,
          error: String(err),
          latency_ms: 0,
        },
      }));
    } finally {
      setTesting((prev) => ({ ...prev, [app]: false }));
    }
  };

  const handleViewConfig = async (app: CliAppType, fileName?: string) => {
    const status = statuses[app];
    if (!status) return;
    const targetFile = fileName || status.files[0];
    try {
      const content = await request<string>("get_cli_config_content", {
        appType: app,
        fileName: targetFile,
      });
      setEditorState({
        app,
        fileName: targetFile,
        allFiles: status.files,
        content,
        isGenerated: false,
        isValid: true,
      });
    } catch (err) {
      console.error("Failed to read config:", err);
    }
  };

  const handleViewSwitchFile = async (fileName: string) => {
    if (!editorState) return;
    try {
      const content = await request<string>("get_cli_config_content", {
        appType: editorState.app,
        fileName,
      });
      setEditorState({
        ...editorState,
        fileName,
        content,
        isGenerated: false,
        isValid: true,
      });
    } catch (err) {
      console.error("Failed to read config:", err);
    }
  };

  return (
    <div className="space-y-4">
      <div className="px-1 flex items-center justify-between">
        <div className="flex items-center gap-2 text-base-content/60">
          <Terminal size={14} />
          <span className="text-xs font-bold uppercase tracking-widest">
            {t("cli.configSync")}
          </span>
        </div>
        <p className="text-xs text-base-content/40 italic">
          {t("cli.configSyncDesc")}
        </p>
      </div>

      <div className="rounded-2xl border border-base-300 bg-base-100 p-4 shadow-sm">
        <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
          <div>
            <div className="flex items-center gap-2 text-sm font-semibold">
              <Sparkles size={16} className="text-primary" />
              <span>{text("同步模型", "Models to sync")}</span>
            </div>
            <p className="mt-1 text-xs text-base-content/55">
              {text(
                "这里显示为“站点名::模型名”，便于区分来源；真正写入 OpenCode、Claude Code 等 CLI 的仍然只是裸模型名。",
                "The list shows site name plus model, but CLI configs still receive bare model names only.",
              )}
            </p>
          </div>
          <div className="rounded-xl border border-base-300 bg-base-200/50 px-3 py-2 text-xs text-base-content/70">
            {useAllDiscoveredModels ? (
              <span>
                {text(
                  "当前将同步全部已发现站点模型",
                  "All discovered site-model bindings will be synced",
                )}
              </span>
            ) : selectedModelKeys.length > 0 ? (
              <span>
                {text("已选择", "Selected")} {selectedModelKeys.length}{" "}
                {text("个站点模型", "site-model bindings")}
              </span>
            ) : (
              <span>
                {text(
                  "当前未选择模型，点击同步时只会写入地址和密钥。",
                  "No models are selected, so syncing only writes the endpoint and API key.",
                )}
              </span>
            )}
            {defaultModel && (
              <div className="mt-1 font-mono text-[11px] text-base-content/55 truncate max-w-[18rem]">
                {text("默认模型", "Default")}: {defaultModel}
              </div>
            )}
          </div>
        </div>

        <div className="mt-4 flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
          <label className="input input-bordered input-sm flex items-center gap-2 w-full lg:max-w-md">
            <Search size={14} className="text-base-content/45" />
            <input
              className="grow bg-transparent"
              placeholder={text("搜索站点或模型", "Search sites or models")}
              value={modelFilter}
              onChange={(event) => setModelFilter(event.target.value)}
            />
          </label>

          <div className="flex flex-wrap items-center gap-2">
            <button
              className="btn btn-outline btn-sm"
              onClick={selectVisibleModels}
              type="button"
              disabled={visibleModelOptions.length === 0}
            >
              {text("添加当前结果", "Add visible")}
            </button>
            <button
              className="btn btn-ghost btn-sm"
              onClick={requestSyncAllModels}
              type="button"
            >
              {text("同步全部", "Use all")}
            </button>
            <button
              className="btn btn-ghost btn-sm text-error"
              onClick={requestClearSelectedModels}
              type="button"
              disabled={selectedModelKeys.length === 0}
            >
              {text("全部移除", "Remove all")}
            </button>
          </div>
        </div>

        {selectedBindingOptions.length > 0 && (
          <div className="mt-4 rounded-2xl border border-base-300 bg-base-200/30 p-3">
            <div className="flex items-center justify-between gap-3">
              <div>
                <div className="text-xs font-semibold uppercase tracking-[0.12em] text-base-content/55">
                  {text("已选模型", "Selected models")}
                </div>
                <div className="mt-1 text-xs text-base-content/45">
                  {text(
                    "这些模型会作为当前 CLI 同步目标保留下来，重启后也会按已保存配置回显。",
                    "These bindings stay as your current CLI sync targets and will be restored from saved config.",
                  )}
                </div>
              </div>
              <div className="text-xs text-base-content/50">
                {selectedBindingOptions.length} {text("项", "items")}
              </div>
            </div>
            <div className="mt-3 flex flex-wrap gap-2">
              {selectedBindingOptions.map((option) => (
                <button
                  key={`selected-${option.key}`}
                  type="button"
                  onClick={() => toggleSelectedModel(option.key)}
                  className="inline-flex max-w-full items-center gap-2 rounded-full border border-primary/20 bg-primary/10 px-3 py-1.5 text-xs text-primary transition hover:border-primary/40 hover:bg-primary/15"
                  title={option.displayLabel}
                >
                  <span className="truncate font-mono">{option.displayLabel}</span>
                  <X size={12} className="shrink-0" />
                </button>
              ))}
            </div>
          </div>
        )}

        {duplicateSiteBindings.length > 0 && (
          <div className="mt-3 rounded-xl border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning-content/80">
            <div className="flex items-start gap-2">
              <TriangleAlert size={14} className="mt-0.5 shrink-0" />
              <div>
                {text(
                  "发现同名模型来自多个站点。写入 CLI 时仍然只能使用裸模型名，因此会以你最先选中的站点为准。",
                  "Some bare model names exist on multiple sites. CLI tools still use the bare model only, so the first selected site wins.",
                )}
                <div className="mt-1 font-mono text-[11px] break-words">
                  {duplicateSiteBindings.join(", ")}
                </div>
              </div>
            </div>
          </div>
        )}

        {modelOptions.length === 0 ? (
          <div className="mt-4 rounded-xl border border-dashed border-base-300 px-4 py-5 text-sm text-base-content/50">
            {text(
              "暂时还没有发现可同步的模型，请先让代理加载模型列表。",
              "No syncable models were discovered yet. Load your proxy model list first.",
            )}
          </div>
        ) : (
          <div className="mt-4 grid max-h-56 grid-cols-1 gap-2 overflow-y-auto pr-1 md:grid-cols-2 xl:grid-cols-3">
            {visibleModelOptions.map((option) => {
              const selected = selectedModelKeys.includes(option.key);
              return (
                <button
                  key={option.key}
                  type="button"
                  onClick={() => toggleSelectedModel(option.key)}
                  className={cn(
                    "flex items-center justify-between gap-3 rounded-xl border px-3 py-2 text-left transition-colors",
                    selected
                      ? "border-primary bg-primary/10 text-primary"
                      : "border-base-300 bg-base-200/30 hover:border-primary/35 hover:bg-base-200/60",
                  )}
                  title={option.displayLabel}
                >
                  <span className="min-w-0">
                    <span className="block truncate font-mono text-xs">
                      {option.displayLabel}
                    </span>
                    <span className="block truncate text-[11px] text-base-content/45">
                      {option.accountSelector || option.accountId}
                    </span>
                  </span>
                  <span
                    className={cn(
                      "shrink-0 rounded-full px-2 py-0.5 text-[10px] font-semibold",
                      selected
                        ? "bg-primary text-primary-content"
                        : "bg-base-200 text-base-content/50",
                    )}
                  >
                    {selected ? text("已选", "Selected") : text("加入", "Add")}
                  </span>
                </button>
              );
            })}
          </div>
        )}
      </div>

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
        {CLI_APPS.map((app) => (
          <CliAppCard
            key={app}
            app={app}
            name={nameMap[app]}
            icon={iconMap[app]}
            status={statuses[app]}
            isLoading={loading[app]}
            isSyncing={syncing[app]}
            isTesting={testing[app]}
            probeResult={probeResults[app]}
            onViewConfig={() => handleViewConfig(app)}
            onRestore={() => handleRestore(app)}
            onTest={() => handleTest(app)}
            onSync={() => handleSync(app)}
          />
        ))}
      </div>

      {confirmDialog && (
        <div className="fixed inset-0 z-[90] flex items-center justify-center bg-slate-950/35 px-4 backdrop-blur-[2px]">
          <div className="w-full max-w-md rounded-3xl border border-base-300 bg-base-100 p-5 shadow-2xl">
            <div className="flex items-start justify-between gap-4">
              <div>
                <div className="text-xs font-semibold uppercase tracking-[0.16em] text-base-content/45">
                  {text("请确认", "Please confirm")}
                </div>
                <h3 className="mt-1 text-lg font-semibold text-base-content">
                  {confirmDialog.title}
                </h3>
              </div>
              <button
                type="button"
                className="btn btn-ghost btn-sm btn-circle"
                onClick={() => setConfirmDialog(null)}
                aria-label={text("关闭", "Close")}
              >
                <X size={16} />
              </button>
            </div>
            <p className="mt-3 text-sm leading-6 text-base-content/70">
              {confirmDialog.message}
            </p>
            <div className="mt-5 flex items-center justify-end gap-3">
              <button
                type="button"
                className="btn btn-ghost"
                onClick={() => setConfirmDialog(null)}
              >
                {text("取消", "Cancel")}
              </button>
              <button
                type="button"
                className={cn(
                  "btn",
                  confirmDialog.tone === "danger" ? "btn-error" : "btn-primary",
                )}
                onClick={confirmDialog.onConfirm}
              >
                {confirmDialog.confirmLabel}
              </button>
            </div>
          </div>
        </div>
      )}

      {editorState && (
        <ConfigEditorModal
          editorState={editorState}
          appIcon={iconMap[editorState.app]}
          appName={nameMap[editorState.app]}
          syncing={syncing[editorState.app]}
          onClose={() => setEditorState(null)}
          onChange={(value) => {
            const isJson = editorState.fileName.endsWith(".json");
            setEditorState({
              ...editorState,
              content: value,
              isValid: isJson ? isValidJson(value) : true,
            });
          }}
          onSwitchFile={(fileName) =>
            editorState.isGenerated
              ? handleSwitchFile(fileName)
              : handleViewSwitchFile(fileName)
          }
          onApply={handleApply}
        />
      )}
    </div>
  );
}
