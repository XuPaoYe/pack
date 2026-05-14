import { Download, RotateCw } from "lucide-react";
import type { ForceUpdateState } from "../hooks/useUpdater";

function formatBytes(bytes: number) {
  if (bytes <= 0) return "0 KB";
  const units = ["B", "KB", "MB", "GB"];
  const index = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / 1024 ** index;
  return `${value >= 10 || index === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[index]}`;
}

export function ForceUpdateModal({
  state,
  onInstall,
  onViewLogs,
}: {
  state: ForceUpdateState;
  onInstall: () => void;
  onViewLogs: () => void;
}) {
  const progressPercent = state.totalBytes
    ? Math.min(100, Math.round((state.downloadedBytes / state.totalBytes) * 100))
    : 0;
  const isWorking = state.phase === "downloading" || state.phase === "installing";
  const retryLabel =
    state.errorKind === "install"
      ? "重新安装"
      : state.errorKind === "download"
        ? "重新下载"
        : "重试升级";
  const helperText =
    state.phase === "error"
      ? state.errorKind === "install"
        ? "安装阶段失败，通常是文件占用或系统权限拦截。关闭相关进程后再试。"
        : state.errorKind === "download"
          ? "下载阶段失败，检查网络或稍后再试。"
          : "本次升级未完成，请重试。"
      : "Super AI 必须升级到新版本后才能继续使用。";

  return (
    <div className="modal-overlay force-update-overlay">
      <aside
        className="force-update-panel modal-content"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="force-update-title"
      >
        <div className="force-update-icon">
          <Download size={24} strokeWidth={2.1} />
        </div>
        <div className="force-update-copy">
          <h2 id="force-update-title">发现新版本</h2>
          <p>
            Super AI {state.version} 已可用，当前版本 {state.currentVersion}。{helperText}
          </p>
        </div>
        {(state.phase === "downloading" || state.phase === "installing") && (
          <div className="force-update-progress" aria-label="升级进度">
            <div>
              <span>{state.phase === "installing" ? "正在安装" : "正在下载"}</span>
              <b>{state.totalBytes ? `${progressPercent}%` : formatBytes(state.downloadedBytes)}</b>
            </div>
            <i>
              <span style={{ width: state.totalBytes ? `${progressPercent}%` : "35%" }} />
            </i>
          </div>
        )}
        {state.phase === "error" && (
          <p className="force-update-error">{state.error ?? "升级失败，请重试。"}</p>
        )}
        <div className="force-update-actions">
          {state.phase === "error" && (
            <button className="secondary force-update-button" onClick={onViewLogs} disabled={isWorking}>
              查看日志
            </button>
          )}
          <button className="primary force-update-button" onClick={onInstall} disabled={isWorking}>
            {state.phase === "error" ? <RotateCw size={18} /> : <Download size={18} />}
            {state.phase === "error" ? retryLabel : isWorking ? "升级中" : "立即升级"}
          </button>
        </div>
      </aside>
    </div>
  );
}
