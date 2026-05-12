import { useCallback, useRef, useState } from "react";
import {
  createLogId,
  pruneAppLogs,
  type AppLogEntry,
  type NoticeTone,
} from "../lib/appLogs";

export type Notice = { tone: NoticeTone; text: string };

export type UseNoticeOptions = {
  sanitize: (text: string) => string;
  initialLogs: AppLogEntry[];
  /** toast 自动消失时长（ms），默认 7000。 */
  timeoutMs?: number;
};

/**
 * 集中管理 toast 通知 + 应用日志。
 *
 * 设计上故意把日志和 toast 绑在一起：每条 toast 都会同步落到日志，
 * 这样用户错过弹窗也能在日志面板里回看，且 `showNotice` 是唯一入口、
 * 调用方不必同时维护两份。
 */
export function useNotice({ sanitize, initialLogs, timeoutMs = 7000 }: UseNoticeOptions) {
  const [notice, setNotice] = useState<Notice | null>(null);
  const [appLogs, setAppLogs] = useState<AppLogEntry[]>(initialLogs);
  const noticeTimer = useRef<number | null>(null);

  const appendAppLog = useCallback(
    (tone: NoticeTone, text: string) => {
      const sanitizedText = sanitize(text);
      setAppLogs((current) =>
        pruneAppLogs([
          {
            id: createLogId(),
            tone,
            text: sanitizedText,
            createdAt: Date.now(),
          },
          ...current,
        ]),
      );
    },
    [sanitize],
  );

  const closeNotice = useCallback(() => {
    if (noticeTimer.current) window.clearTimeout(noticeTimer.current);
    noticeTimer.current = null;
    setNotice(null);
  }, []);

  const showNotice = useCallback(
    (tone: NoticeTone, text: string) => {
      const sanitizedText = sanitize(text);
      appendAppLog(tone, sanitizedText);
      if (noticeTimer.current) window.clearTimeout(noticeTimer.current);
      setNotice({ tone, text: sanitizedText });
      noticeTimer.current = window.setTimeout(() => {
        setNotice(null);
        noticeTimer.current = null;
      }, timeoutMs);
    },
    [appendAppLog, sanitize, timeoutMs],
  );

  return {
    notice,
    closeNotice,
    showNotice,
    appendAppLog,
    appLogs,
    setAppLogs,
  };
}
