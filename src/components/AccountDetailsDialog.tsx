import { useEffect } from "react";
import { Clock, X } from "lucide-react";
import clsx from "clsx";
import type { ManagedAccount, QuotaBucket } from "../lib/authParser";
import { formatDateTime, formatResetTime } from "../lib/time";
import { useVirtualScrollbar } from "../hooks/useVirtualScrollbar";

function quotaTone(percent?: number): "good" | "warn" | "bad" | "unknown" {
  if (percent === undefined) return "unknown";
  if (percent >= 50) return "good";
  if (percent >= 20) return "warn";
  return "bad";
}

function quotaBucketTone(bucket: QuotaBucket): "good" | "warn" | "bad" | "unknown" {
  return quotaTone(bucket.remainingPercent);
}

function bucketLabel(bucket: QuotaBucket) {
  const label = bucket.displayName ?? bucket.window;
  return label.toUpperCase() === "WEEKLY" ? "WEEKLY" : label.toUpperCase();
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

  const { ref: bodyRef, state: scrollbar, update: updateScrollbar } =
    useVirtualScrollbar<HTMLDivElement>();

  const groups = account.quota?.groups?.filter((group) => group.buckets.length > 0) ?? [];
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
            <h2 id="account-details-title">配额详情</h2>
            <span className="account-details-email">{account.email}</span>
            {tier && <span className="account-details-tier">{tier}</span>}
          </div>
          <button className="icon-button" aria-label="关闭" onClick={onClose}>
            <X size={18} strokeWidth={1.8} />
          </button>
        </header>
        <div className="account-details-body-wrap">
          <div className="account-details-body" ref={bodyRef} onScroll={updateScrollbar}>
            <div className="account-details-tabs" aria-label="配额视图">
              <span className="account-details-tab active">详细配额</span>
            </div>
            {groups.length === 0 ? (
              <div className="account-details-empty">
                {account.quota?.error ?? "暂无分组配额，请先刷新账号"}
              </div>
            ) : (
              <div className="quota-group-list">
                {groups.map((group, groupIndex) => (
                  <section className="quota-group-card" key={`${group.displayName}-${groupIndex}`}>
                    <div className="quota-group-head">
                      <h3>{group.displayName}</h3>
                      {group.description && <p>{group.description}</p>}
                    </div>
                    <div className="quota-group-buckets">
                      {group.buckets.map((bucket, bucketIndex) => {
                        const tone = quotaBucketTone(bucket);
                        const reset = formatResetTime(bucket.resetAt);
                        const resetTitle = formatDateTime(bucket.resetAt);
                        return (
                          <article className={clsx("quota-detail-tile", tone)} key={`${bucket.bucketId}-${bucketIndex}`}>
                            <div className="quota-detail-tile-head">
                              <div className="quota-detail-tile-name">
                                <strong>{bucketLabel(bucket)}</strong>
                              </div>
                              <span className="quota-detail-percent">
                                {bucket.remainingPercent === undefined ? "N/A" : `${bucket.remainingPercent}%`}
                              </span>
                            </div>
                            <div className={clsx("quota-detail-bar", tone)}>
                              <i style={{ width: `${bucket.remainingPercent ?? 0}%` }} />
                            </div>
                            <div className="quota-detail-foot" title={resetTitle ?? undefined}>
                              <Clock size={11} strokeWidth={1.8} />
                              <span>重置时间：{reset ?? "未知"}</span>
                            </div>
                          </article>
                        );
                      })}
                    </div>
                  </section>
                ))}
              </div>
            )}
          </div>
          <div className={clsx("account-scrollbar", "in-dialog", scrollbar.visible && "visible")} aria-hidden="true">
            <i style={{ height: scrollbar.height, transform: `translateY(${scrollbar.top}px)` }} />
          </div>
        </div>
      </aside>
    </div>
  );
}
