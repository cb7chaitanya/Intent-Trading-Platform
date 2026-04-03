import React, { useEffect, useRef } from "react";
import { createChart, CandlestickSeries } from "lightweight-charts";
import { getCandles } from "@/lib/api";

interface CandlestickChartProps {
  marketId: string;
  interval?: string;
}

const CandlestickChart: React.FC<CandlestickChartProps> = ({
  marketId,
  interval = "1m",
}) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<ReturnType<typeof createChart> | null>(null);

  useEffect(() => {
    if (!containerRef.current) return;

    const isDark = document.documentElement.classList.contains("dark");

    const chart = createChart(containerRef.current, {
      width: containerRef.current.clientWidth,
      height: 320,
      layout: {
        background: { color: "transparent" },
        textColor: isDark ? "#94a3b8" : "#475569",
        fontFamily: "Inter, system-ui, sans-serif",
        fontSize: 11,
      },
      grid: {
        vertLines: { color: isDark ? "#1e293b" : "#e2e8f0" },
        horzLines: { color: isDark ? "#1e293b" : "#e2e8f0" },
      },
      crosshair: {
        vertLine: { labelBackgroundColor: "#2d5fff" },
        horzLine: { labelBackgroundColor: "#2d5fff" },
      },
      rightPriceScale: { borderColor: isDark ? "#1e293b" : "#e2e8f0" },
      timeScale: { borderColor: isDark ? "#1e293b" : "#e2e8f0" },
    });

    const series = chart.addSeries(CandlestickSeries, {
      upColor: "#22c55e",
      downColor: "#ef4444",
      wickUpColor: "#22c55e",
      wickDownColor: "#ef4444",
      borderVisible: false,
    });

    chartRef.current = chart;

    // Load data
    getCandles(marketId, interval)
      .then((candles) => {
        if (candles && candles.length > 0) {
          const data = candles.map((c: any) => ({
            time: Math.floor(new Date(c.bucket).getTime() / 1000),
            open: c.open,
            high: c.high,
            low: c.low,
            close: c.close,
          }));
          series.setData(data);
          chart.timeScale().fitContent();
        }
      })
      .catch(() => {});

    // Resize observer
    const observer = new ResizeObserver(() => {
      if (containerRef.current) {
        chart.applyOptions({ width: containerRef.current.clientWidth });
      }
    });
    observer.observe(containerRef.current);

    return () => {
      observer.disconnect();
      chart.remove();
    };
  }, [marketId, interval]);

  return (
    <div className="card !p-0 overflow-hidden">
      <div className="px-4 py-3 border-b flex items-center justify-between">
        <h3 className="text-sm font-semibold">Price Chart</h3>
        <div className="flex gap-1">
          {["1m", "5m", "15m", "1h", "1d"].map((iv) => (
            <span
              key={iv}
              className={`px-2 py-0.5 rounded text-xs cursor-pointer transition-colors ${
                iv === interval
                  ? "bg-brand-600 text-white"
                  : "text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-surface-2"
              }`}
            >
              {iv}
            </span>
          ))}
        </div>
      </div>
      <div ref={containerRef} className="w-full" />
    </div>
  );
};

export default CandlestickChart;
