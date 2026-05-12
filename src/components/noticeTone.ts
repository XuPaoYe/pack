import { BadgeCheck, CircleAlert, Info } from "lucide-react";
import type { NoticeTone } from "../lib/appLogs";

// 拆到独立模块，避免 NoticeToast 同时 export 组件 + 常量，
// 触发 react-refresh/only-export-components 规则。
export const noticeToneConfig: Record<NoticeTone, { icon: typeof Info; label: string }> = {
  success: { icon: BadgeCheck, label: "成功" },
  error: { icon: CircleAlert, label: "错误" },
  info: { icon: Info, label: "提示" },
};
