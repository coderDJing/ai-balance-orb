import { useCallback, useEffect, useMemo, useState } from "react";
import { Eye, EyeOff, Power, RefreshCw, Save, Settings, X } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

const REFRESH_INTERVAL_MS = 60_000;

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

type ClientConfig = {
  hasAccessToken: boolean;
  endpointUrl?: string;
  userId?: string;
};

type BalanceSnapshot = {
  configured: boolean;
  remaining: number | null;
  username?: string | null;
  group?: string | null;
  requestCount?: number | null;
  refreshedAtMs: number;
};

type Status = "idle" | "loading" | "ok" | "error" | "setup";

async function runCommand<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (typeof window === "undefined" || !window.__TAURI_INTERNALS__) {
    return runPreviewCommand<T>(command, args);
  }

  return invoke<T>(command, args);
}

async function runPreviewCommand<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (command === "load_config") {
    return {
      hasAccessToken: false,
      endpointUrl: "",
      userId: "",
    } as T;
  }

  if (command === "query_balance") {
    return {
      configured: false,
      remaining: null,
      username: null,
      group: null,
      requestCount: null,
      refreshedAtMs: Date.now(),
    } as T;
  }

  if (command === "save_config") {
    return {
      hasAccessToken: Boolean(args?.accessToken),
      endpointUrl: String(args?.endpointUrl || ""),
      userId: String(args?.userId || ""),
    } as T;
  }

  return undefined as T;
}

function App() {
  const [config, setConfig] = useState<ClientConfig>({
    hasAccessToken: false,
  });
  const [snapshot, setSnapshot] = useState<BalanceSnapshot | null>(null);
  const [status, setStatus] = useState<Status>("idle");
  const [error, setError] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [accessToken, setAccessToken] = useState("");
  const [endpointUrl, setEndpointUrl] = useState("");
  const [userId, setUserId] = useState("");
  const [showToken, setShowToken] = useState(false);
  const [now, setNow] = useState(Date.now());

  const refresh = useCallback(async () => {
    setStatus("loading");
    setError("");

    try {
      const nextSnapshot = await runCommand<BalanceSnapshot>("query_balance");
      setSnapshot(nextSnapshot);

      if (!nextSnapshot.configured) {
        setStatus("setup");
        setSettingsOpen(true);
        return;
      }

      setStatus("ok");
    } catch (err) {
      setStatus("error");
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  useEffect(() => {
    let mounted = true;

    async function bootstrap() {
      const loaded = await runCommand<ClientConfig>("load_config");
      if (!mounted) return;

      setConfig(loaded);
      setEndpointUrl(loaded.endpointUrl || "");
      setUserId(loaded.userId || "");
      if (!loaded.hasAccessToken) {
        setSettingsOpen(true);
      }
      await refresh();
    }

    bootstrap().catch((err) => {
      if (!mounted) return;
      setStatus("error");
      setError(err instanceof Error ? err.message : String(err));
    });

    return () => {
      mounted = false;
    };
  }, [refresh]);

  useEffect(() => {
    const timer = window.setInterval(refresh, REFRESH_INTERVAL_MS);
    return () => window.clearInterval(timer);
  }, [refresh]);

  useEffect(() => {
    const ticker = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(ticker);
  }, []);

  const remainingText = useMemo(() => {
    if (typeof snapshot?.remaining !== "number") return "--";
    return snapshot.remaining.toLocaleString("zh-CN", {
      minimumFractionDigits: 2,
      maximumFractionDigits: 6,
    });
  }, [snapshot]);

  const elapsed = snapshot?.refreshedAtMs
    ? Math.max(0, now - Number(snapshot.refreshedAtMs))
    : 0;
  const progress = Math.min(1, elapsed / REFRESH_INTERVAL_MS);
  const secondsLeft = Math.max(
    0,
    Math.ceil((REFRESH_INTERVAL_MS - elapsed) / 1000),
  );

  async function saveSettings() {
    setStatus("loading");
    setError("");

    try {
      const saved = await runCommand<ClientConfig>("save_config", {
        endpointUrl: endpointUrl.trim(),
        accessToken: accessToken.trim() || undefined,
        userId: userId.trim(),
      });
      setConfig(saved);
      setAccessToken("");
      setSettingsOpen(false);
      await refresh();
    } catch (err) {
      setStatus("error");
      setError(err instanceof Error ? err.message : String(err));
    }
  }

  function hideWindow() {
    runCommand("hide_window").catch(() => undefined);
  }

  const healthText =
    status === "loading"
      ? "刷新中"
      : status === "error"
        ? "异常"
        : status === "setup"
          ? "待配置"
          : "在线";

  return (
    <main className="shell">
      <div className="orb-window" data-tauri-drag-region>
        <header className="topbar" data-tauri-drag-region>
          <div className="brand" data-tauri-drag-region>
            <span className="brand-mark" />
            <span>AI Balance</span>
          </div>
          <div className="actions">
            <button
              className="icon-button"
              type="button"
              title="刷新"
              onClick={refresh}
              disabled={status === "loading"}
            >
              <RefreshCw size={15} />
            </button>
            <button
              className="icon-button"
              type="button"
              title="设置"
              onClick={() => setSettingsOpen((open) => !open)}
            >
              <Settings size={15} />
            </button>
            <button
              className="icon-button close"
              type="button"
              title="隐藏到托盘"
              onClick={hideWindow}
            >
              <X size={15} />
            </button>
          </div>
        </header>

        <section className="balance-stage" data-tauri-drag-region>
          <div
            className="progress-ring"
            style={{ "--progress": progress } as React.CSSProperties}
          >
            <div className="ring-core">
              <Power size={18} />
            </div>
          </div>
          <div className="balance-copy" data-tauri-drag-region>
            <span className={`status-pill ${status}`}>{healthText}</span>
            <strong className={snapshot?.remaining && snapshot.remaining < 0 ? "negative" : ""}>
              {remainingText}
            </strong>
            <div className="meta-line">
              <span>{snapshot?.username || config.userId || "未绑定"}</span>
              <span>{snapshot?.group || "default"}</span>
              <span>{status === "ok" ? `${secondsLeft}s` : "--"}</span>
            </div>
          </div>
        </section>

        {error && <div className="error-line">{error}</div>}

        {settingsOpen && (
          <section className="settings-panel">
            <div className="field endpoint">
              <label htmlFor="endpoint-url">API Endpoint</label>
              <input
                id="endpoint-url"
                value={endpointUrl}
                placeholder="https://example.com/api/user/self"
                onChange={(event) => setEndpointUrl(event.currentTarget.value)}
              />
            </div>

            <div className="field">
              <label htmlFor="access-token">Access Token</label>
              <div className="input-wrap">
                <input
                  id="access-token"
                  type={showToken ? "text" : "password"}
                  value={accessToken}
                  placeholder={
                    config.hasAccessToken ? "已保存，留空不改" : "安全设置里生成"
                  }
                  onChange={(event) => setAccessToken(event.currentTarget.value)}
                />
                <button
                  className="inline-icon"
                  type="button"
                  title={showToken ? "隐藏" : "显示"}
                  onClick={() => setShowToken((show) => !show)}
                >
                  {showToken ? <EyeOff size={14} /> : <Eye size={14} />}
                </button>
              </div>
            </div>

            <div className="field compact">
              <label htmlFor="user-id">User ID</label>
              <input
                id="user-id"
                value={userId}
                placeholder="User ID"
                onChange={(event) => setUserId(event.currentTarget.value)}
              />
            </div>

            <button className="save-button" type="button" onClick={saveSettings}>
              <Save size={14} />
              <span>保存</span>
            </button>
          </section>
        )}
      </div>
    </main>
  );
}

export default App;
