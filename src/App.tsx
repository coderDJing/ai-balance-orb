import { useCallback, useEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { Save, Settings, X } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./App.css";

const DEFAULT_REFRESH_INTERVAL_MS = 60_000;
const PREVIEW_BALANCE = 1286.734;
const FLAP_DURATION_MS = 288;
const FLAP_STEP_MS = 305;
const INITIAL_FLAP_STEP_MS = 45;
const MAX_COUNTER_STEPS = 160;
const MAIN_WINDOW_MIN_WIDTH = 220;
const MAIN_WINDOW_MAX_WIDTH = 520;
const NON_DRAG_SELECTOR =
  "button, input, textarea, select, label, a, [role='button'], [data-window-no-drag]";

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

type ClientConfig = {
  hasAccessToken: boolean;
  accessToken?: string;
  endpointUrl?: string;
  userId?: string;
  refreshIntervalSecs: number;
  autostartEnabled: boolean;
};

type BalanceSnapshot = {
  configured: boolean;
  remaining: number | null;
  username?: string | null;
  group?: string | null;
  requestCount?: number | null;
  refreshedAtMs: number;
};

type WindowKind = "main" | "settings";
type SaveStatus = "idle" | "saving" | "saved" | "error";
type ActiveFlip = {
  from: string;
  to: string;
  version: number;
};

type FlipTransition = ActiveFlip & {
  delayMs: number;
  index: number;
};

function isTauriRuntime() {
  return typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);
}

function previewWindowKind(): WindowKind {
  const params = new URLSearchParams(window.location.search);
  return params.get("window") === "settings" ? "settings" : "main";
}

async function runCommand<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauriRuntime()) {
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
      hasAccessToken: true,
      accessToken: "",
      endpointUrl: "https://example.com",
      userId: "preview",
      refreshIntervalSecs: 60,
    } as T;
  }

  if (command === "query_balance") {
    return {
      configured: true,
      remaining: PREVIEW_BALANCE,
      username: null,
      group: null,
      requestCount: null,
      refreshedAtMs: Date.now(),
    } as T;
  }

  if (command === "save_config") {
    return {
      hasAccessToken: Boolean(args?.accessToken),
      accessToken: String(args?.accessToken || ""),
      endpointUrl: String(args?.endpointUrl || ""),
      userId: String(args?.userId || ""),
      refreshIntervalSecs: Number(args?.refreshIntervalSecs) || 60,
    } as T;
  }

  if (command === "resize_main_window") {
    return undefined as T;
  }

  return undefined as T;
}

function App() {
  const [windowKind, setWindowKind] = useState<WindowKind>(previewWindowKind);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    setWindowKind(getCurrentWindow().label === "settings" ? "settings" : "main");
  }, []);

  return windowKind === "settings" ? <SettingsWindow /> : <BalanceWindow />;
}

function startWindowDrag(event: React.MouseEvent<HTMLElement>) {
  if (!isTauriRuntime() || event.button !== 0 || event.buttons !== 1) return;

  const target = event.target;
  if (!(target instanceof HTMLElement) || target.closest(NON_DRAG_SELECTOR)) {
    return;
  }

  void getCurrentWindow().startDragging().catch(() => undefined);
}

function formatBalance(value: number | null): string {
  if (value == null) return "--";
  if (!Number.isFinite(value)) return "--";
  return value.toFixed(3);
}

function measureBalanceWindowWidth(text: string) {
  const contentWidth = text.split("").reduce((width, char) => {
    if (isFlipDigit(char)) return width + 28;
    if (char === ".") return width + 9;
    if (char === "-") return width + 18;
    return width + 8;
  }, 60);
  const gapWidth = Math.max(0, text.length - 1);
  return Math.max(
    MAIN_WINDOW_MIN_WIDTH,
    Math.min(MAIN_WINDOW_MAX_WIDTH, Math.ceil(contentWidth + gapWidth)),
  );
}

function isFlipDigit(char: string) {
  return char >= "0" && char <= "9";
}

function parseDisplayNumber(text: string) {
  const value = Number(text);
  return Number.isFinite(value) ? value : null;
}

function parseDisplayUnits(text: string) {
  const value = parseDisplayNumber(text);
  return value == null ? null : Math.round(value * 1000);
}

function hasSameStaticLayout(startText: string, targetText: string) {
  if (startText.length !== targetText.length) return false;

  for (let index = 0; index < targetText.length; index += 1) {
    if (isFlipDigit(targetText[index])) continue;
    if (startText[index] !== targetText[index]) return false;
  }

  return true;
}

function previousCharAt(previousText: string, currentIndex: number, currentLength: number) {
  const previousIndex = currentIndex - (currentLength - previousText.length);
  return previousIndex >= 0 ? previousText[previousIndex] || "" : "";
}

function alignStartText(previousText: string, targetText: string, initial: boolean) {
  const targetChars = targetText.split("");
  return targetChars
    .map((char, index) => {
      if (!isFlipDigit(char)) return char;
      if (initial) return "0";

      const previousChar = previousCharAt(previousText, index, targetChars.length);
      return isFlipDigit(previousChar) ? previousChar : "0";
    })
    .join("");
}

function nextDigit(current: number, increasing: boolean) {
  return (current + (increasing ? 1 : -1) + 10) % 10;
}

function buildDirectDigitTransitions(startText: string, targetText: string, versionBase: number) {
  const transitions: FlipTransition[] = [];
  let delaySteps = 0;

  for (let index = targetText.length - 1; index >= 0; index -= 1) {
    const targetChar = targetText[index];
    if (!isFlipDigit(targetChar) || startText[index] === targetChar) continue;

    transitions.push({
      delayMs: delaySteps * INITIAL_FLAP_STEP_MS,
      from: startText[index],
      index,
      to: targetChar,
      version: versionBase + transitions.length,
    });
    delaySteps += 1;
  }

  return transitions;
}

function buildCounterTransitions(
  startText: string,
  targetText: string,
  increasing: boolean,
  versionBase: number,
) {
  const transitions: FlipTransition[] = [];
  const startUnits = parseDisplayUnits(startText);
  const targetUnits = parseDisplayUnits(targetText);

  if (
    startUnits == null ||
    targetUnits == null ||
    !hasSameStaticLayout(startText, targetText)
  ) {
    return buildDirectDigitTransitions(startText, targetText, versionBase);
  }

  const stepCount = Math.abs(targetUnits - startUnits);
  const digitIndexes = startText
    .split("")
    .map((char, index) => (isFlipDigit(char) ? index : -1))
    .filter((index) => index >= 0);

  if (stepCount > MAX_COUNTER_STEPS || digitIndexes.length === 0) {
    return buildDirectDigitTransitions(startText, targetText, versionBase);
  }

  const workingChars = startText.split("");
  for (let step = 0; step < stepCount; step += 1) {
    const delayMs = step * FLAP_STEP_MS;

    for (let digitCursor = digitIndexes.length - 1; digitCursor >= 0; digitCursor -= 1) {
      const index = digitIndexes[digitCursor];
      const currentDigit = Number(workingChars[index]);
      const next = nextDigit(currentDigit, increasing);
      const wrapped = increasing
        ? currentDigit === 9 && next === 0
        : currentDigit === 0 && next === 9;

      transitions.push({
        delayMs,
        from: String(currentDigit),
        index,
        to: String(next),
        version: versionBase + transitions.length,
      });

      workingChars[index] = String(next);
      if (!wrapped) break;
    }
  }

  return transitions;
}

function BalanceWindow() {
  const [balanceText, setBalanceText] = useState(formatBalance(null));
  const [displayText, setDisplayText] = useState(formatBalance(null));
  const [activeFlips, setActiveFlips] = useState<Record<number, ActiveFlip>>({});
  const [queryStatus, setQueryStatus] = useState<"loading" | "error" | "ready">("loading");
  const [errorMsg, setErrorMsg] = useState("");
  const displayTextRef = useRef(formatBalance(null));
  const flipTimersRef = useRef<number[]>([]);
  const flipVersionRef = useRef(0);
  const windowWidthRef = useRef(0);
  const refreshIntervalMs = useRef(DEFAULT_REFRESH_INTERVAL_MS);
  const promptedSetupRef = useRef(false);

  const clearFlipTimers = useCallback(() => {
    for (const timer of flipTimersRef.current) {
      window.clearTimeout(timer);
    }
    flipTimersRef.current = [];
  }, []);

  const showSettings = useCallback(async () => {
    await runCommand("show_settings_window").catch(() => undefined);
  }, []);

  const animateBalance = useCallback(
    (nextText: string) => {
      clearFlipTimers();
      setActiveFlips({});

      const previousText = displayTextRef.current;
      const previousValue = parseDisplayNumber(previousText);
      const nextValue = parseDisplayNumber(nextText);
      const shouldAnimateDecrease =
        previousValue != null && nextValue != null && nextValue < previousValue;

      if (!shouldAnimateDecrease) {
        displayTextRef.current = nextText;
        setDisplayText(nextText);
        return;
      }

      const startText = alignStartText(previousText, nextText, false);
      const versionBase = (flipVersionRef.current += 1000);
      const transitions = buildCounterTransitions(startText, nextText, false, versionBase);

      displayTextRef.current = startText;
      setDisplayText(startText);

      if (transitions.length === 0) {
        displayTextRef.current = nextText;
        setDisplayText(nextText);
        return;
      }

      const workingChars = startText.split("");
      for (const transition of transitions) {
        const startTimer = window.setTimeout(() => {
          setActiveFlips((flips) => ({
            ...flips,
            [transition.index]: {
              from: transition.from,
              to: transition.to,
              version: transition.version,
            },
          }));
        }, transition.delayMs);

        const endTimer = window.setTimeout(() => {
          workingChars[transition.index] = transition.to;
          const updatedText = workingChars.join("");
          displayTextRef.current = updatedText;
          setDisplayText(updatedText);
          setActiveFlips((flips) => {
            if (flips[transition.index]?.version !== transition.version) {
              return flips;
            }
            const nextFlips = { ...flips };
            delete nextFlips[transition.index];
            return nextFlips;
          });
        }, transition.delayMs + FLAP_DURATION_MS);

        flipTimersRef.current.push(startTimer, endTimer);
      }

      const finalDelay =
        Math.max(...transitions.map((transition) => transition.delayMs)) +
        FLAP_DURATION_MS +
        30;
      const finalTimer = window.setTimeout(() => {
        displayTextRef.current = nextText;
        setDisplayText(nextText);
        setActiveFlips({});
      }, finalDelay);
      flipTimersRef.current.push(finalTimer);
    },
    [clearFlipTimers],
  );

  const refresh = useCallback(async () => {
    try {
      const snapshot = await runCommand<BalanceSnapshot>("query_balance");

      if (!snapshot.configured) {
        if (!promptedSetupRef.current) {
          promptedSetupRef.current = true;
          await showSettings();
        }
        return;
      }

      const nextText = formatBalance(snapshot.remaining);
      setBalanceText(nextText);
      animateBalance(nextText);
      setQueryStatus("ready");
      setErrorMsg("");
    } catch {
      // 静默处理错误
    }
  }, [animateBalance, showSettings]);

  useEffect(() => {
    let mounted = true;

    async function bootstrap() {
      const loaded = await runCommand<ClientConfig>("load_config");
      if (!mounted) return;

      if (!loaded.hasAccessToken || !loaded.endpointUrl || !loaded.userId) {
        promptedSetupRef.current = true;
        setQueryStatus("error");
        setErrorMsg("请先完成配置");
        await showSettings();
        return;
      }
      refreshIntervalMs.current = (loaded.refreshIntervalSecs || 60) * 1000;

      setQueryStatus("loading");
      try {
        const snapshot = await runCommand<BalanceSnapshot>("query_balance");
        if (!mounted) return;

        if (!snapshot.configured) {
          promptedSetupRef.current = true;
          setQueryStatus("error");
          setErrorMsg("请先完成配置");
          await showSettings();
          return;
        }

        const nextText = formatBalance(snapshot.remaining);
        setBalanceText(nextText);
        animateBalance(nextText);
        setQueryStatus("ready");
        setErrorMsg("");
      } catch (err) {
        if (mounted) {
          setQueryStatus("error");
          setErrorMsg(err instanceof Error ? err.message : "接口连接失败");
        }
      }
    }

    bootstrap().catch(() => undefined);

    return () => {
      mounted = false;
    };
  }, [refresh, showSettings]);

  useEffect(() => {
    const timer = window.setInterval(refresh, refreshIntervalMs.current);
    return () => window.clearInterval(timer);
  }, [refresh]);

  useEffect(() => clearFlipTimers, [clearFlipTimers]);

  useEffect(() => {
    if (!isTauriRuntime()) return;

    const width = measureBalanceWindowWidth(balanceText);
    if (windowWidthRef.current === width) return;
    windowWidthRef.current = width;
    runCommand("resize_main_window", { width }).catch(() => undefined);
  }, [balanceText]);

  useEffect(() => {
    if (!isTauriRuntime()) return;

    let unlisten: (() => void) | undefined;
    listen<ClientConfig>("config-saved", async (event) => {
      promptedSetupRef.current = false;
      setQueryStatus("loading");
      setErrorMsg("");
      if (event.payload?.refreshIntervalSecs) {
        refreshIntervalMs.current = event.payload.refreshIntervalSecs * 1000;
      }
      const win = getCurrentWindow();
      await win.show().catch(() => undefined);
      await win.setFocus().catch(() => undefined);
      await refresh();
    })
      .then((handler) => {
        unlisten = handler;
      })
      .catch(() => undefined);

    return () => {
      unlisten?.();
    };
  }, [refresh]);

  const balanceChars = displayText.split("");
  const flapScale = Math.max(0.82, Math.min(1, 13 / Math.max(balanceChars.length, 1)));

  return (
    <main className="shell main-shell" onMouseDown={startWindowDrag} onDoubleClick={showSettings}>
      <div className="orb-window">
        <button
          className={`icon-button settings-btn${queryStatus !== "ready" ? " always-visible" : ""}`}
          type="button"
          title="设置"
          onClick={showSettings}
        >
          <Settings size={14} />
        </button>
        <div className="balance-number">
          {queryStatus === "loading" ? (
            <div className="loading-indicator">查询中...</div>
          ) : (
            <div
              className="flap-machine"
              aria-label={`余额 ${balanceText}`}
              style={{ "--flap-scale": flapScale } as CSSProperties}
            >
              {balanceChars.map((char, index) => {
                if (!isFlipDigit(char)) {
                  const staticClassName = `flap-static${char === "," ? " separator" : ""}${
                    char === "." ? " decimal" : ""
                  }${char === "-" ? " minus" : ""}`;
                  return (
                    <span key={`static-${index}-${char}`} className={staticClassName}>
                      {char}
                    </span>
                  );
                }

                const activeFlip = activeFlips[index];
                const topChar = activeFlip?.to || char;
                const bottomChar = activeFlip?.from || char;
                const flapStyle = {
                  "--flap-duration": `${FLAP_DURATION_MS}ms`,
                } as CSSProperties;

                return (
                  <span
                    key={`flap-${index}`}
                    className={`flap-card${activeFlip ? " is-flipping" : ""}`}
                    style={flapStyle}
                    aria-hidden="true"
                  >
                    <span className="flap-half flap-top current">
                      <span className="flap-glyph">{topChar}</span>
                    </span>
                    <span className="flap-half flap-bottom current">
                      <span className="flap-glyph">{bottomChar}</span>
                    </span>
                    {activeFlip && (
                      <>
                        <span
                          key={`old-${activeFlip.version}`}
                          className="flap-half flap-top old"
                        >
                          <span className="flap-glyph">{activeFlip.from}</span>
                        </span>
                        <span
                          key={`next-${activeFlip.version}`}
                          className="flap-half flap-bottom next"
                        >
                          <span className="flap-glyph">{activeFlip.to}</span>
                        </span>
                      </>
                    )}
                  </span>
                );
              })}
            </div>
          )}
        </div>
        {queryStatus === "error" && errorMsg && (
          <div className="error-line">{errorMsg}</div>
        )}
      </div>
    </main>
  );
}

function SettingsWindow() {
  const [config, setConfig] = useState<ClientConfig>({ hasAccessToken: false, refreshIntervalSecs: 60, autostartEnabled: true });
  const [accessToken, setAccessToken] = useState("");
  const [endpointUrl, setEndpointUrl] = useState("");
  const [userId, setUserId] = useState("");
  const [refreshIntervalSecs, setRefreshIntervalSecs] = useState("60");
  const [autostartEnabled, setAutostartEnabled] = useState(true);
  const [saveStatus, setSaveStatus] = useState<SaveStatus>("idle");
  const [error, setError] = useState("");
  const [setupHint, setSetupHint] = useState("");
  const [appVersion, setAppVersion] = useState("");

  useEffect(() => {
    let mounted = true;

    runCommand<ClientConfig>("load_config")
      .then((loaded) => {
        if (!mounted) return;
        setConfig(loaded);
        setEndpointUrl(loaded.endpointUrl || "");
        setAccessToken(loaded.accessToken || "");
        setUserId(loaded.userId || "");
        setRefreshIntervalSecs(String(loaded.refreshIntervalSecs || 60));
        setAutostartEnabled(loaded.autostartEnabled ?? true);
      })
      .catch((err) => {
        if (!mounted) return;
        setSaveStatus("error");
        setError(err instanceof Error ? err.message : String(err));
      });

    return () => {
      mounted = false;
    };
  }, []);

  useEffect(() => {
    if (!isTauriRuntime()) return;

    let unlisten: (() => void) | undefined;
    listen<string>("setup-required", (event) => {
      setSetupHint(event.payload);
      // 3 秒后自动消失
      setTimeout(() => setSetupHint(""), 3000);
    })
      .then((handler) => {
        unlisten = handler;
      })
      .catch(() => undefined);

    return () => {
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!isTauriRuntime()) return;

    getVersion()
      .then((version) => {
        setAppVersion(version);
      })
      .catch(() => undefined);
  }, []);

  async function saveSettings() {
    setSaveStatus("saving");
    setError("");

    try {
      const interval = Math.max(10, Math.min(3600, parseInt(refreshIntervalSecs) || 60));
      const saved = await runCommand<ClientConfig>("save_config", {
        endpointUrl: endpointUrl.trim(),
        accessToken: accessToken.trim() || undefined,
        userId: userId.trim(),
        refreshIntervalSecs: interval,
        autostartEnabled,
      });
      setConfig(saved);
      setAccessToken(saved.accessToken || "");
      setRefreshIntervalSecs(String(saved.refreshIntervalSecs));
      setAutostartEnabled(saved.autostartEnabled);
      setSaveStatus("saved");

      // 保存成功后关闭设置窗口
      await hideWindow();
    } catch (err) {
      setSaveStatus("error");
      setError(err instanceof Error ? err.message : String(err));
    }
  }

  return (
    <main className="shell settings-shell">
      <section className="settings-window">
        <header className="topbar settings-topbar" data-window-drag-handle onMouseDown={startWindowDrag}>
          <div className="brand">
            <span className="brand-mark" />
            <span>Settings</span>
          </div>
          <button
            className="icon-button close"
            type="button"
            title="隐藏设置"
            onClick={hideWindow}
          >
            <X size={15} />
          </button>
        </header>

        <div className="settings-body">
          {setupHint && <div className="setup-hint">{setupHint}</div>}
          <label className="settings-field" htmlFor="endpoint-url">
            <span>Base URL</span>
            <input
              id="endpoint-url"
              data-window-no-drag
              value={endpointUrl}
              placeholder="https://example.com"
              onChange={(event) => {
                setEndpointUrl(event.currentTarget.value);
                setSaveStatus("idle");
              }}
            />
          </label>

          <label className="settings-field" htmlFor="access-token">
            <span>Access Token</span>
            <input
              id="access-token"
              data-window-no-drag
              value={accessToken}
              placeholder={config.hasAccessToken ? "已保存，留空不改" : "Access Token"}
              onChange={(event) => {
                setAccessToken(event.currentTarget.value);
                setSaveStatus("idle");
              }}
            />
          </label>

          <label className="settings-field" htmlFor="user-id">
            <span>User ID</span>
            <input
              id="user-id"
              data-window-no-drag
              value={userId}
              placeholder="User ID"
              onChange={(event) => {
                setUserId(event.currentTarget.value);
                setSaveStatus("idle");
              }}
            />
          </label>

          <label className="settings-field" htmlFor="refresh-interval">
            <span>刷新间隔（秒）</span>
            <input
              id="refresh-interval"
              type="number"
              data-window-no-drag
              value={refreshIntervalSecs}
              placeholder="60"
              min={10}
              max={3600}
              onChange={(event) => {
                setRefreshIntervalSecs(event.currentTarget.value);
                setSaveStatus("idle");
              }}
            />
          </label>

          <label className="settings-field settings-field-toggle" htmlFor="autostart">
            <span>开机自启动</span>
            <input
              id="autostart"
              type="checkbox"
              data-window-no-drag
              checked={autostartEnabled}
              onChange={(event) => {
                setAutostartEnabled(event.currentTarget.checked);
                setSaveStatus("idle");
              }}
            />
          </label>

          {error && <div className="settings-error">{error}</div>}

          <div className="settings-actions">
            <button className="secondary-button" type="button" onClick={hideWindow}>
              <span>隐藏</span>
            </button>
            <button
              className="save-button"
              type="button"
              onClick={saveSettings}
              disabled={saveStatus === "saving"}
            >
              <Save size={14} />
              <span>{saveStatus === "saving" ? "保存中" : "保存"}</span>
            </button>
          </div>

          {appVersion && (
            <div className="settings-version">
              v{appVersion}
            </div>
          )}
        </div>
      </section>
    </main>
  );
}

function hideWindow() {
  runCommand("hide_window").catch(() => undefined);
}

export default App;
