import { useEffect, useRef, useState } from "react";
import type { SystemPush, SystemSnapshot } from "./types";

const WS_URL = "ws://127.0.0.1:9600";
const STALE_MS = 5000;
const BACKOFF_INITIAL_MS = 1000;
const BACKOFF_MAX_MS = 10000;
const FRESHNESS_POLL_MS = 1000;

export interface SystemSocketState {
  /** 最近一次收到的系统快照；从未连上过 daemon 时为 null。 */
  snapshot: SystemSnapshot | null;
  /** 已连接且数据新鲜（ts_ms 距今 < 5s）。false 时 UI 应显示重连中降级态。 */
  live: boolean;
}

/**
 * 订阅 daemon 的 WebSocket 推送（CLAUDE.md §3：shell 自身不做任何采集）。
 * 断线后按 1s 起倍增、上限 10s 的退避自动重连。
 */
export function useSystemSocket(): SystemSocketState {
  const [snapshot, setSnapshot] = useState<SystemSnapshot | null>(null);
  const [live, setLive] = useState(false);
  const snapshotRef = useRef<SystemSnapshot | null>(null);
  const socketRef = useRef<WebSocket | null>(null);
  const reconnectDelayRef = useRef(BACKOFF_INITIAL_MS);
  const reconnectTimerRef = useRef<number | undefined>(undefined);
  const unmountedRef = useRef(false);

  useEffect(() => {
    unmountedRef.current = false;

    const scheduleReconnect = () => {
      socketRef.current = null;
      if (unmountedRef.current) return;
      const delay = reconnectDelayRef.current;
      reconnectDelayRef.current = Math.min(delay * 2, BACKOFF_MAX_MS);
      reconnectTimerRef.current = window.setTimeout(connect, delay);
    };

    function connect() {
      const socket = new WebSocket(WS_URL);
      socketRef.current = socket;

      socket.onopen = () => {
        reconnectDelayRef.current = BACKOFF_INITIAL_MS;
      };

      socket.onmessage = (event) => {
        if (typeof event.data !== "string") return;
        try {
          const push = JSON.parse(event.data) as SystemPush;
          if (push.type === "system") {
            snapshotRef.current = push.data;
            setSnapshot(push.data);
          }
        } catch {
          // 忽略无法解析的消息，等待下一条推送。
        }
      };

      // 连接失败或断开都会触发 close（规范保证），重连逻辑集中在这里处理。
      socket.onclose = scheduleReconnect;
    }

    connect();

    return () => {
      unmountedRef.current = true;
      window.clearTimeout(reconnectTimerRef.current);
      const socket = socketRef.current;
      socketRef.current = null;
      if (socket) {
        socket.onclose = null;
        socket.close();
      }
    };
  }, []);

  useEffect(() => {
    const tick = () => {
      const current = snapshotRef.current;
      const fresh = current !== null && Date.now() - current.ts_ms < STALE_MS;
      const connected = socketRef.current?.readyState === WebSocket.OPEN;
      setLive(connected && fresh);
    };
    tick();
    const interval = window.setInterval(tick, FRESHNESS_POLL_MS);
    return () => window.clearInterval(interval);
  }, []);

  return { snapshot, live };
}
