import type { MouseEvent } from "react";
import { BadgeCheck, Info, LockKeyhole, Monitor } from "lucide-react";
import logoUrl from "../assets/logo.svg";

type AboutInfo = { name: string; version: string } | null;

type Props = {
  appName: string;
  isPublicBuild: boolean;
  aboutInfo: AboutInfo;
  onOpenStore: (event: MouseEvent<HTMLAnchorElement>) => void;
};

export function AboutPanelBody({ appName, isPublicBuild, aboutInfo, onOpenStore }: Props) {
  return (
    <div className="settings-body about-body">
      <section className="setting-row about-app-row">
        <div className="setting-copy">
          <img className="about-icon" src={logoUrl} alt="" draggable={false} />
          <div>
            <strong>{aboutInfo?.name ?? appName}</strong>
            <p>本地 ChatGPT / Antigravity / {appName} 账号管理</p>
          </div>
        </div>
      </section>

      <div className="about-info-grid">
        <section className="about-info-card">
          <div className="setting-copy">
            <BadgeCheck size={18} />
            <div>
              <strong>应用版本</strong>
              <p>{aboutInfo?.version ?? "加载中…"}</p>
            </div>
          </div>
        </section>

        <section className="about-info-card">
          <div className="setting-copy">
            <LockKeyhole size={18} />
            <div>
              <strong>构建模式</strong>
              <p>{isPublicBuild ? "公开版" : "完全版"}</p>
            </div>
          </div>
        </section>

        <section className="about-info-card">
          <div className="setting-copy">
            <Monitor size={18} />
            <div>
              <strong>运行平台</strong>
              <p>{typeof navigator !== "undefined" ? navigator.platform || "—" : "—"}</p>
            </div>
          </div>
        </section>

        <section className="about-info-card">
          <div className="setting-copy">
            <Info size={18} />
            <div>
              <strong>应用标识</strong>
              <p>cn.talentisan.super-ai</p>
            </div>
          </div>
        </section>
      </div>

      <p className="about-note">
        更多说明见{" "}
        <a href="https://ai.talentisan.cn/" onClick={onOpenStore}>
          ai.talentisan.cn
        </a>
        。所有账号凭证仅保存在本机。
      </p>
    </div>
  );
}
