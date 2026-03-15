import { useState, useEffect, useCallback, useMemo, type ReactNode } from "react";
import {
  Bot,
  CodeXml,
  Cpu,
  Globe,
  Search,
  Sparkles,
  Terminal,
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

interface CliSyncCardProps {
  proxyUrl: string;
  apiKey: string;
  proxyPort: number;
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

export default function CliSyncCard({
  proxyUrl,
  apiKey,
  proxyPort,
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
  const [availableModels, setAvailableModels] = useState<string[]>([]);
  const [selectedModels, setSelectedModels] = useState<string[]>([]);
  const [modelFilter, setModelFilter] = useState("");
  const [editorState, setEditorState] = useState<EditorState | null>(null);
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

  const visibleModels = useMemo(() => {
    const keyword = modelFilter.trim().toLowerCase();
    if (!keyword) return availableModels;
    return availableModels.filter((model) => model.toLowerCase().includes(keyword));
  }, [availableModels, modelFilter]);

  const effectiveModels = useMemo(
    () => (selectedModels.length > 0 ? selectedModels : availableModels),
    [availableModels, selectedModels],
  );

  const defaultModel = effectiveModels[0] ?? null;

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
    request<string[]>("get_available_models")
      .then((models) => setAvailableModels(dedupe(models ?? [])))
      .catch(() => {});
  }, []);

  useEffect(() => {
    setSelectedModels((prev) =>
      prev.filter((model) => availableModels.includes(model)),
    );
  }, [availableModels]);

  useEffect(() => {
    CLI_APPS.forEach(checkStatus);
  }, [checkStatus]);

  const toggleSelectedModel = (model: string) => {
    setSelectedModels((prev) =>
      prev.includes(model)
        ? prev.filter((item) => item !== model)
        : [...prev, model],
    );
  };

  const selectVisibleModels = () => {
    setSelectedModels((prev) => dedupe([...prev, ...visibleModels]));
  };

  const clearSelectedModels = () => {
    setSelectedModels([]);
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
                "在这里选中的模型会一起写入 OpenCode、Claude Code 等 CLI 配置。单模型工具会优先使用第一个选中的模型。",
                "Selected models will be written into compatible CLI configs together. Single-model CLIs use the first selected model as their default.",
              )}
            </p>
          </div>
          <div className="rounded-xl border border-base-300 bg-base-200/50 px-3 py-2 text-xs text-base-content/70">
            {selectedModels.length > 0 ? (
              <span>
                {text("已选择", "Selected")} {selectedModels.length}{" "}
                {text("个模型", "models")}
              </span>
            ) : (
              <span>{text("未手动选择时将同步全部已发现模型", "No manual selection means sync all discovered models")}</span>
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
              placeholder={text("搜索模型", "Search models")}
              value={modelFilter}
              onChange={(event) => setModelFilter(event.target.value)}
            />
          </label>

          <div className="flex flex-wrap items-center gap-2">
            <button
              className="btn btn-outline btn-sm"
              onClick={selectVisibleModels}
              type="button"
              disabled={visibleModels.length === 0}
            >
              {text("添加当前结果", "Add visible")}
            </button>
            <button
              className="btn btn-ghost btn-sm"
              onClick={clearSelectedModels}
              type="button"
            >
              {text("同步全部", "Use all")}
            </button>
          </div>
        </div>

        {availableModels.length === 0 ? (
          <div className="mt-4 rounded-xl border border-dashed border-base-300 px-4 py-5 text-sm text-base-content/50">
            {text(
              "暂时还没有发现可同步的模型，请先让代理加载模型列表。",
              "No syncable models were discovered yet. Load your proxy model list first.",
            )}
          </div>
        ) : (
          <div className="mt-4 grid max-h-52 grid-cols-1 gap-2 overflow-y-auto pr-1 md:grid-cols-2 xl:grid-cols-3">
            {visibleModels.map((model) => {
              const selected = selectedModels.includes(model);
              return (
                <button
                  key={model}
                  type="button"
                  onClick={() => toggleSelectedModel(model)}
                  className={cn(
                    "flex items-center justify-between gap-3 rounded-xl border px-3 py-2 text-left transition-colors",
                    selected
                      ? "border-primary bg-primary/10 text-primary"
                      : "border-base-300 bg-base-200/30 hover:border-primary/35 hover:bg-base-200/60",
                  )}
                  title={model}
                >
                  <span className="min-w-0 truncate font-mono text-xs">
                    {model}
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
