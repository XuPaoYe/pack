import type { Notice } from "../hooks/useNotice";
import { noticeToneConfig } from "./noticeTone";

export function NoticeToast({
  notice,
  onClose,
  sanitize,
}: {
  notice: Notice;
  onClose: () => void;
  sanitize: (text: string) => string;
}) {
  const config = noticeToneConfig[notice.tone];
  const Icon = config.icon;

  return (
    <div className="toast" data-tone={notice.tone} role="status" aria-live="polite">
      <Icon className="toast-icon" size={16} aria-hidden="true" />
      <span className="toast-text">
        <b>{config.label}</b>
        {sanitize(notice.text)}
      </span>
      <button onClick={onClose} aria-label="关闭提示">
        ×
      </button>
    </div>
  );
}
