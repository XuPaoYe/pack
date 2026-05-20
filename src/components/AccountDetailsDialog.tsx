import { useEffect } from "react";
import { Clock, X } from "lucide-react";
import clsx from "clsx";
import type { ManagedAccount, QuotaMetric } from "../lib/authParser";
import { formatDateTime, formatResetTime } from "../lib/time";

function quotaTone(percent?: number): "good" | "warn" | "bad" | "unknown" {
  if (percent === undefined) return "unknown";
  if (percent >= 50) return "good";
  if (percent >= 20) return "warn";
  return "bad";
}

function sortMetrics(metrics: QuotaMetric[]): QuotaMetric[] {
  return [...metrics].sort((a, b) => {
    const aName = (a.modelName ?? a.label).toLowerCase();
    const bName = (b.modelName ?? b.label).toLowerCase();
    return aName.localeCompare(bName);
  });
}

export function AccountDetailsDialog({
  account,
  onClose,
}: {
  account: ManagedAccount;
  onClose: () => void;
}) {
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onClose]);

  const metrics = account.quota?.metrics ? sortMetrics(account.quota.metrics) : [];
  const tier = account.plan ?? account.planType;

  return (
    <div className="modal-overlay account-details-overlay" onClick={onClose}>
      <aside
        className="modal-content account-details-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="account-details-title"
        onClick={(event) => event.stopPropagation()}
      >
        <header className="account-details-header">
          <div className="account-details-heading">
            <h2 id="account-details-title">账号明细</h2>
            <span className="account-details-email">{account.email}</span>
            {tier && <span className="account-details-tier">{tier}</span>}
          </div>
          <button className="icon-button" aria-label="关闭" onClick={onClose}>
            <X size={18} strokeWidth={1.8} />
          </button>
        </header>
        <div className="account-details-body">
          {metrics.length === 0 ? (
            <div className="account-details-empty">
              {account.quota?.error ?? "暂无明细数据"}
            </div>
          ) : (
            <div className="account-details-grid">
              {metrics.map((metric) => {
                const tone = quotaTone(metric.remainingPercent);
                const reset = formatResetTime(metric.resetAt);
                const resetTitle = formatDateTime(metric.resetAt);
                return (
                  <article className={clsx("quota-detail-tile", tone)} key={metric.key}>
                    <div className="quota-detail-tile-head">
                      <div className="quota-detail-tile-name">
                        <strong>{metric.displayName ?? metric.label}</strong>
                        {metric.modelName && metric.modelName !== (metric.displayName ?? metric.label) && (
                          <span className="quota-detail-tile-model">{metric.modelName}</span>
                        )}
                      </div>
                      <span className="quota-detail-percent">
                        {metric.remainingPercent === undefined ? "N/A" : `${metric.remainingPercent}%`}
                      </span>
                    </div>
                    {metric.thinkingBudget !== undefined && (
                      <span className="thinking-budget-pill">
                        Thinking Budget: {metric.thinkingBudget}
                      </span>
                    )}
                    <div className={clsx("quota-detail-bar", tone)}>
                      <i style={{ width: `${metric.remainingPercent ?? 0}%` }} />
                    </div>
                    <div className="quota-detail-foot" title={resetTitle ?? undefined}>
                      <Clock size={11} strokeWidth={1.8} />
                      <span>重置时间：{reset ?? "未知"}</span>
                    </div>
                  </article>
                );
              })}
            </div>
          )}
        </div>
      </aside>
    </div>
  );
}
