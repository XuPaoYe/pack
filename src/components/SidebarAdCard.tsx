import type { MouseEvent } from "react";
import { ExternalLink } from "lucide-react";

type Props = {
  onOpenStore: (event: MouseEvent<HTMLAnchorElement>) => void;
};

export function SidebarAdCard({ onOpenStore }: Props) {
  return (
    <a className="ad-card" href="https://ai.talentisan.cn/" onClick={onOpenStore}>
      <div>
        <span>AI 权益补给站</span>
        <strong>购买 AI 到 Super Store</strong>
      </div>
      <ExternalLink size={18} />
    </a>
  );
}
