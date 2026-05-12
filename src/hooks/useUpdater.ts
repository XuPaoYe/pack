import { useCallback, useEffect, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";

export type ForceUpdateState = {
  update: Update;
  phase: "ready" | "downloading" | "installing" | "error";
  version: string;
  currentVersion: string;
  downloadedBytes: number;
  totalBytes: number | null;
  error?: string;
};

export type UpdaterHooks = {
  appendAppLog: (tone: "info" | "error", text: string) => void;
  showError: (text: string) => void;
  sanitize: (text: string) => string;
};

// 启动检测 + 15 分钟轮询，避免必须重启 App 才能感知发布。
const POLL_INTERVAL_MS = 15 * 60 * 1000;

export function useUpdater({ appendAppLog, showError, sanitize }: UpdaterHooks) {
  const [forceUpdate, setForceUpdate] = useState<ForceUpdateState | null>(null);

  useEffect(() => {
    if (import.meta.env.DEV || !isTauri()) return;
    let isCancelled = false;
    // 第一次检测失败时只在 UI 弹一次错误；后续轮询失败仅写日志，避免反复打扰。
    let hasShownErrorOnce = false;
    // updater 未配置一旦确认就不再重试，省掉每 15 分钟一次的无意义 IO。
    let updaterUnconfigured = false;

    async function pollForUpdate(reason: "launch" | "interval") {
      if (isCancelled || updaterUnconfigured) return;
      try {
        const update = await check();
        if (!update || isCancelled) return;
        // 已经检出过同一次更新（或正在下载/安装），不要覆盖当前状态：
        // 否则下载进度条会被重置回 ready，弹窗也会从启动那个变成轮询那个。
        setForceUpdate((current) => {
          if (current) return current;
          return {
            update,
            phase: "ready",
            version: update.version,
            currentVersion: update.currentVersion,
            downloadedBytes: 0,
            totalBytes: null,
          };
        });
      } catch (error) {
        if (isCancelled) return;
        const message = String(error);
        const isUpdaterUnconfigured =
          message.includes("plugins > updater doesn't exist") ||
          (message.includes("updater") && message.includes("configuration"));
        if (isUpdaterUnconfigured) {
          updaterUnconfigured = true;
          if (reason === "launch") {
            appendAppLog("info", "远程升级未配置，已跳过更新检测。");
          }
          return;
        }
        if (!hasShownErrorOnce) {
          hasShownErrorOnce = true;
          showError(`检测更新失败：${message}`);
        } else {
          appendAppLog("info", `定时更新检测失败：${message}`);
        }
      }
    }

    void pollForUpdate("launch");
    const timer = window.setInterval(() => {
      void pollForUpdate("interval");
    }, POLL_INTERVAL_MS);

    return () => {
      isCancelled = true;
      window.clearInterval(timer);
    };
  }, [appendAppLog, showError]);

  const installForceUpdate = useCallback(async () => {
    if (!forceUpdate) return;
    let downloadedBytes = 0;

    try {
      setForceUpdate((current) =>
        current
          ? {
              ...current,
              phase: "downloading",
              downloadedBytes: 0,
              totalBytes: null,
              error: undefined,
            }
          : current,
      );

      const handleDownloadEvent = (event: DownloadEvent) => {
        if (event.event === "Started") {
          downloadedBytes = 0;
          setForceUpdate((current) =>
            current
              ? {
                  ...current,
                  phase: "downloading",
                  downloadedBytes: 0,
                  totalBytes: event.data.contentLength ?? null,
                }
              : current,
          );
          return;
        }

        if (event.event === "Progress") {
          downloadedBytes += event.data.chunkLength;
          setForceUpdate((current) =>
            current
              ? {
                  ...current,
                  downloadedBytes,
                }
              : current,
          );
          return;
        }

        setForceUpdate((current) =>
          current
            ? {
                ...current,
                phase: "installing",
                downloadedBytes: current.totalBytes ?? current.downloadedBytes,
              }
            : current,
        );
      };

      await forceUpdate.update.downloadAndInstall(handleDownloadEvent);
      await relaunch();
    } catch (error) {
      setForceUpdate((current) =>
        current
          ? {
              ...current,
              phase: "error",
              error: sanitize(`升级失败：${String(error)}`),
            }
          : current,
      );
    }
  }, [forceUpdate, sanitize]);

  return { forceUpdate, installForceUpdate };
}
