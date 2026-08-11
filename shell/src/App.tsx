import { useEffect, useState } from "react";
import { useSystemSocket } from "./useSystemSocket";
import type { SystemSnapshot } from "./types";
import "./App.css";

function formatMetric(value: number | null | undefined, unit: string): string {
  return value === null || value === undefined ? "N/A" : `${Math.round(value)}${unit}`;
}

function ClockCard() {
  const [now, setNow] = useState(() => new Date());

  useEffect(() => {
    const timer = window.setInterval(() => setNow(new Date()), 1000);
    return () => window.clearInterval(timer);
  }, []);

  const hh = String(now.getHours()).padStart(2, "0");
  const mm = String(now.getMinutes()).padStart(2, "0");
  const dateLabel = now.toLocaleDateString("zh-CN", {
    month: "long",
    day: "numeric",
    weekday: "long",
  });

  return (
    <section className="card clock-card">
      <div className="clock-time">
        {hh}:{mm}
      </div>
      <div className="clock-date">{dateLabel}</div>
    </section>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="metric">
      <div className="metric-value">{value}</div>
      <div className="metric-label">{label}</div>
    </div>
  );
}

function MetricsCard({ snapshot }: { snapshot: SystemSnapshot | null }) {
  return (
    <section className="card metrics-card">
      <div className="metrics-grid">
        <Metric label="CPU 利用率" value={formatMetric(snapshot?.cpu_usage, "%")} />
        <Metric label="CPU 温度" value={formatMetric(snapshot?.cpu_temp, "°C")} />
        <Metric label="GPU 利用率" value={formatMetric(snapshot?.gpu_usage, "%")} />
        <Metric label="GPU 温度" value={formatMetric(snapshot?.gpu_temp, "°C")} />
      </div>
    </section>
  );
}

export default function App() {
  const { snapshot, live } = useSystemSocket();

  return (
    <div className="shell-root">
      {!live && <div className="status-banner">重连中…</div>}
      <ClockCard />
      <MetricsCard snapshot={snapshot} />
    </div>
  );
}
