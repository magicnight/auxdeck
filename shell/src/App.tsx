import { useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties, PointerEvent as ReactPointerEvent } from "react";
import { useSystemSocket } from "./useSystemSocket";
import type {
  AppUsageEntry,
  AppUsageSnapshot,
  SystemSnapshot,
  WeatherSnapshot,
  WidgetPlacement,
} from "./types";
import "./App.css";

/** 翻页手势判定所需的最小纵向位移（CSS px），避免误触发（CLAUDE.md §9 手势仲裁）。 */
const SWIPE_THRESHOLD_PX = 40;

function formatMetric(value: number | null | undefined, unit: string): string {
  return value === null || value === undefined ? "N/A" : `${Math.round(value)}${unit}`;
}

/** "X 小时 Y 分" 格式（应用使用时长卡）。 */
function formatDuration(totalSeconds: number): string {
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  return `${hours} 小时 ${minutes} 分`;
}

/** 对比文案对齐 Nexus 观感（CLAUDE.md §12）：昨日为 0 显示「今日新增」，否则百分比涨跌。 */
function formatComparison(entry: AppUsageEntry): string {
  if (entry.yesterday_secs === 0) return "今日新增";
  const deltaPct = ((entry.today_secs - entry.yesterday_secs) / entry.yesterday_secs) * 100;
  const rounded = Math.round(Math.abs(deltaPct));
  return deltaPct >= 0 ? `相比昨日 提升 ${rounded}%` : `相比昨日 下降 ${rounded}%`;
}

/** widget 在网格上的占位换算为 CSS Grid 的 grid-column/grid-row（1-indexed）。 */
function placementStyle(placement: WidgetPlacement): CSSProperties {
  return {
    gridColumn: `${placement.x + 1} / span ${placement.w}`,
    gridRow: `${placement.y + 1} / span ${placement.h}`,
  };
}

function ClockCard({ style }: { style: CSSProperties }) {
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
    <section className="card clock-card" style={style}>
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

function MetricsCard({ style, snapshot }: { style: CSSProperties; snapshot: SystemSnapshot | null }) {
  return (
    <section className="card metrics-card" style={style}>
      <div className="metrics-grid">
        <Metric label="CPU 利用率" value={formatMetric(snapshot?.cpu_usage, "%")} />
        <Metric label="CPU 温度" value={formatMetric(snapshot?.cpu_temp, "°C")} />
        <Metric label="GPU 利用率" value={formatMetric(snapshot?.gpu_usage, "%")} />
        <Metric label="GPU 温度" value={formatMetric(snapshot?.gpu_temp, "°C")} />
      </div>
    </section>
  );
}

function WeatherCard({ style, weather }: { style: CSSProperties; weather: WeatherSnapshot | null }) {
  if (!weather) {
    return (
      <section className="card weather-card weather-card--empty" style={style}>
        <div className="weather-empty-title">天气未配置</div>
        <div className="weather-empty-hint">请在 config.toml 中配置天气数据源与位置</div>
      </section>
    );
  }

  return (
    <section className="card weather-card" style={style}>
      <div className="weather-main">
        <div className="weather-temp">{Math.round(weather.temp_c)}°</div>
        <div className="weather-condition-group">
          <div className="weather-condition">{weather.condition}</div>
          <div className="weather-city">{weather.city}</div>
        </div>
      </div>
      <div className="weather-detail-row">
        <span>
          高 {Math.round(weather.high_c)}° / 低 {Math.round(weather.low_c)}°
        </span>
      </div>
      <div className="weather-detail-row">
        <span>日出 {weather.sunrise}</span>
        <span>日落 {weather.sunset}</span>
      </div>
    </section>
  );
}

function AppUsageCard({ style, usage }: { style: CSSProperties; usage: AppUsageSnapshot | null }) {
  if (!usage || usage.top.length === 0) {
    return (
      <section className="card app-usage-card app-usage-card--empty" style={style}>
        <div className="app-usage-empty-title">暂无使用数据</div>
      </section>
    );
  }

  return (
    <section className="card app-usage-card" style={style}>
      <div className="app-usage-list">
        {usage.top.map((entry) => (
          <div className="app-usage-row" key={entry.name}>
            <div className="app-usage-name">{entry.name}</div>
            <div className="app-usage-stats">
              <span className="app-usage-time">{formatDuration(entry.today_secs)}</span>
              <span className="app-usage-compare">{formatComparison(entry)}</span>
            </div>
          </div>
        ))}
      </div>
    </section>
  );
}

interface WidgetCellProps {
  placement: WidgetPlacement;
  system: SystemSnapshot | null;
  weather: WeatherSnapshot | null;
  appUsage: AppUsageSnapshot | null;
}

/** 按 kind 分发到具体 widget 卡片；未知 kind 忽略以保证前向兼容（CLAUDE.md §8.1）。 */
function WidgetCell({ placement, system, weather, appUsage }: WidgetCellProps) {
  const style = placementStyle(placement);
  switch (placement.kind) {
    case "clock":
      return <ClockCard style={style} />;
    case "metrics":
      return <MetricsCard style={style} snapshot={system} />;
    case "weather":
      return <WeatherCard style={style} weather={weather} />;
    case "app_usage":
      return <AppUsageCard style={style} usage={appUsage} />;
    default:
      return null;
  }
}

function PageDots({
  count,
  current,
  onSelect,
}: {
  count: number;
  current: number;
  onSelect: (page: number) => void;
}) {
  if (count <= 1) return null;
  return (
    <div className="page-dots">
      {Array.from({ length: count }, (_, index) => (
        <button
          key={index}
          type="button"
          className={`page-dot-hit${index === current ? " active" : ""}`}
          aria-label={`第 ${index + 1} 页`}
          aria-current={index === current ? "true" : undefined}
          onClick={() => onSelect(index)}
        >
          <span className="page-dot" />
        </button>
      ))}
    </div>
  );
}

/**
 * 翻页手势带（CLAUDE.md §9「手势仲裁」）：只有屏幕右缘 24px 宽条带（.gesture-edge，
 * 见 App.css）响应上下滑动切页；条带之外的区域（含内嵌网页滚动、shorts 上滑换条等）
 * 完全交给 widget 自身处理，本组件不拦截。
 */
function EdgeGestureBand({
  pageCount,
  currentPage,
  onNavigate,
}: {
  pageCount: number;
  currentPage: number;
  onNavigate: (page: number) => void;
}) {
  const startYRef = useRef<number | null>(null);
  const pointerIdRef = useRef<number | null>(null);

  const resetGesture = () => {
    startYRef.current = null;
    pointerIdRef.current = null;
  };

  const handlePointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    startYRef.current = event.clientY;
    pointerIdRef.current = event.pointerId;
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const handlePointerUp = (event: ReactPointerEvent<HTMLDivElement>) => {
    const startY = startYRef.current;
    if (startY === null || pointerIdRef.current !== event.pointerId) {
      resetGesture();
      return;
    }
    const deltaY = event.clientY - startY;
    resetGesture();
    if (Math.abs(deltaY) < SWIPE_THRESHOLD_PX) return;
    // 上滑（deltaY < 0）翻到下一页，下滑翻到上一页；越界处夹到首/末页，不循环。
    const direction = deltaY < 0 ? 1 : -1;
    const next = Math.min(Math.max(currentPage + direction, 0), pageCount - 1);
    if (next !== currentPage) onNavigate(next);
  };

  return (
    <div
      className="gesture-edge"
      onPointerDown={handlePointerDown}
      onPointerUp={handlePointerUp}
      onPointerCancel={resetGesture}
    />
  );
}

type GridCssVars = CSSProperties & {
  "--grid-columns"?: number;
  "--grid-row-height"?: string;
  "--grid-gap"?: string;
};

export default function App() {
  const { snapshot, weather, appUsage, config, live } = useSystemSocket();

  // 页集合来自 config.widgets 的 page 字段去重排序；config 热更新（Config 推送 ->
  // setConfig -> 本 memo 因 config.widgets 引用变化而重算）后自动反映新页数。
  const pages = useMemo(() => {
    const unique = Array.from(new Set(config.widgets.map((widget) => widget.page)));
    unique.sort((a, b) => a - b);
    return unique.length > 0 ? unique : [0];
  }, [config.widgets]);

  const [pageIndex, setPageIndex] = useState(0);

  // 热重载后页数可能变少，越界时收敛到最后一页。
  useEffect(() => {
    setPageIndex((prev) => Math.min(prev, pages.length - 1));
  }, [pages]);

  const gridVars: GridCssVars = {
    "--grid-columns": config.grid.columns,
    "--grid-row-height": `${config.grid.row_height_px}px`,
    "--grid-gap": `${config.grid.gap_px}px`,
  };

  return (
    <div className="shell-root" style={gridVars}>
      {!live && <div className="status-banner">重连中…</div>}
      <div className="page-viewport">
        <div className="page-track" style={{ transform: `translateY(-${pageIndex * 100}%)` }}>
          {pages.map((pageNumber) => (
            <div className="page" key={pageNumber}>
              <div className="widget-grid">
                {config.widgets
                  .filter((widget) => widget.page === pageNumber)
                  .map((widget) => (
                    <WidgetCell
                      key={`${widget.kind}-${widget.page}-${widget.x}-${widget.y}`}
                      placement={widget}
                      system={snapshot}
                      weather={weather}
                      appUsage={appUsage}
                    />
                  ))}
              </div>
            </div>
          ))}
        </div>
        <EdgeGestureBand pageCount={pages.length} currentPage={pageIndex} onNavigate={setPageIndex} />
        <PageDots count={pages.length} current={pageIndex} onSelect={setPageIndex} />
      </div>
    </div>
  );
}
