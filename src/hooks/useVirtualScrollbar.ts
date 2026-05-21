import { useCallback, useEffect, useRef, useState } from "react";

export type VirtualScrollbarState = {
  visible: boolean;
  top: number;
  height: number;
};

/**
 * 复用主账号列表的虚拟滚动条算法（隐藏原生 scrollbar，自己画 thumb）。
 * 返回的 ref 挂到滚动容器，state 渲染到浮在容器外的 thumb。
 * onScroll={update} 必须挂到滚动容器上，因为 ResizeObserver 监听不到 scroll。
 *
 * `active=false` 时跳过订阅，避免关闭的 Modal 留下无用 observer。
 */
export function useVirtualScrollbar<T extends HTMLElement>(active = true) {
  const ref = useRef<T | null>(null);
  const [state, setState] = useState<VirtualScrollbarState>({
    visible: false,
    top: 0,
    height: 0,
  });

  const update = useCallback(() => {
    const element = ref.current;
    if (!element) {
      setState({ visible: false, top: 0, height: 0 });
      return;
    }
    const { clientHeight, scrollHeight, scrollTop } = element;
    const visible = scrollHeight > clientHeight + 1;
    if (!visible) {
      setState({ visible: false, top: 0, height: 0 });
      return;
    }
    const trackInset = 8;
    const trackHeight = Math.max(0, clientHeight - trackInset * 2);
    const height = Math.max(54, Math.round((clientHeight / scrollHeight) * trackHeight));
    const maxScrollTop = Math.max(1, scrollHeight - clientHeight);
    const maxThumbTop = Math.max(0, trackHeight - height);
    const top = trackInset + Math.round((scrollTop / maxScrollTop) * maxThumbTop);
    setState((current) => {
      if (current.visible === visible && current.top === top && current.height === height) {
        return current;
      }
      return { visible, top, height };
    });
  }, []);

  useEffect(() => {
    if (!active) return;
    update();
    const element = ref.current;
    if (!element) return;
    const observer = new ResizeObserver(() => update());
    observer.observe(element);
    for (const child of Array.from(element.children)) observer.observe(child);
    window.addEventListener("resize", update);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", update);
    };
  }, [active, update]);

  return { ref, state, update };
}
