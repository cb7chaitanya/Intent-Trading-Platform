import React, { useEffect, useRef, useState, useCallback } from "react";
import {
  createChart,
  CandlestickSeries,
  HistogramSeries,
} from "lightweight-charts";
import { getCandles } from "@/lib/api";
import { useWebSocket } from "@/contexts/WebSocketProvider";

interface CandlestickChartProps {
  marketId: string;
}

const INTERVALS = ["1m", "5m", "15m", "1h", "4h", "1d"] as const;
type Interval = (typeof INTERVALS)[number];

const CandlestickChart: React.FC<CandlestickChartProps> = ({ marketId }) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<ReturnType<typeof createChart> | null>(null);
  const candleSeriesRef = useRef<any>(null);
  const volumeSeriesRef = useRef<any>(null);
  const [interval, setInterval_] = useState<Interval>("1m");
  const { addListener } = useWebSocket();

  const buildChart = useCallback(() => {
    if (!containerRef.current) return;

    // Clean up previous chart
    if (chartRef.current) {
      chartRef.current.remove();
      chartRef.current = null;
    }

    const isDark = document.documentElement.classList.contains("dark");

    const chart = createChart(containerRef.current, {
      width: containerRef.current.clientWidth,
      height: containerRef.current.clientHeight || 400,
      layout: {
        background: { color: "transparent" },
        textColor: isDark ? "#64748b" : "#475569",
        fontFamily: "Inter, system-ui, sans-serif",
        fontSize: 11,
      },
      grid: {
        vertLines: { color: isDark ? "#1e293b" : "#f1f5f9" },
        horzLines: { color: isDark ? "#1e293b" : "#f1f5f9" },
      },
      crosshair: {
        vertLine: {
          labelBackgroundColor: "#2d5fff",
          color: isDark ? "#334155" : "#cbd5e1",
        },
        horzLine: {
          labelBackgroundColor: "#2d5fff",
          color: isDark ? "#334155" : "#cbd5e1",
        },
      },
      rightPriceScale: {
        borderColor: isDark ? "#1e293b" : "#e2e8f0",
        scaleMargins: { top: 0.1, bottom: 0.25 },
      },
      timeScale: {
        borderColor: isDark ? "#1e293b" : "#e2e8f0",
        timeVisible: true,
        secondsVisible: false,
      },
    });

    const candleSeries = chart.addSeries(CandlestickSeries, {
      upColor: "#22c55e",
      downColor: "#ef4444",
      wickUpColor: "#22c55e",
      wickDownColor: "#ef4444",
      borderVisible: false,
    });

    const volumeSeries = chart.addSeries(HistogramSeries, {
      priceFormat: { type: "volume" },
      priceScaleId: "volume",
    });

    chart.priceScale("volume").applyOptions({
      scaleMargins: { top: 0.8, bottom: 0 },
    });

    chartRef.current = chart;
    candleSeriesRef.current = candleSeries;
    volumeSeriesRef.current = volumeSeries;

    // Load candle data
    getCandles(marketId, interval)
      .then((candles) => {
        if (!candles?.length) return;
        const candleData = candles.map((c: any) => ({
          time: Math.floor(new Date(c.bucket).getTime() / 1000),
          open: c.open,
          high: c.high,
          low: c.low,
          close: c.close,
        }));
        const volumeData = candles.map((c: any) => ({
          time: Math.floor(new Date(c.bucket).getTime() / 1000),
          value: c.volume,
          color: c.close >= c.open ? "rgba(34,197,94,0.3)" : "rgba(239,68,68,0.3)",
        }));
        candleSeries.setData(candleData);
        volumeSeries.setData(volumeData);
        chart.timeScale().fitContent();
      })
      .catch(() => {});

    // Resize observer
    const observer = new ResizeObserver(() => {
      if (containerRef.current) {
        chart.applyOptions({
          width: containerRef.current.clientWidth,
          height: containerRef.current.clientHeight || 400,
        });
      }
    });
    observer.observe(containerRef.current);

    return () => {
      observer.disconnect();
      chart.remove();
      chartRef.current = null;
    };
  }, [marketId, interval]);

  useEffect(() => {
    const cleanup = buildChart();
    return cleanup;
  }, [buildChart]);

  // Live trade updates - update last candle
  useEffect(() => {
    return addListener("Trade", (data) => {
      if (data.market_id !== marketId || !candleSeriesRef.current) return;
      const time = Math.floor(new Date(data.created_at).getTime() / 1000);
      candleSeriesRef.current.update({
        time,
        open: data.price,
        high: data.price,
        low: data.price,
        close: data.price,
      });
    });
  }, [marketId, addListener]);

  return (
    <div className="flex flex-col h-full rounded-xl border bg-surface-1 overflow-hidden">
      {/* Toolbar */}
      <div className="flex items-center justify-between px-3 py-2 border-b">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-[var(--text-muted)]">
          Chart
        </h3>
        <div className="flex gap-0.5">
          {INTERVALS.map((iv) => (
            <button
              key={iv}
              onClick={() => setInterval_(iv)}
              className={`px-2 py-1 rounded text-[11px] font-medium transition-colors ${
                iv === interval
                  ? "bg-brand-600 text-white"
                  : "text-[var(--text-muted)] hover:text-[var(--text-primary)] hover:bg-surface-2"
              }`}
            >
              {iv}
            </button>
          ))}
        </div>
      </div>

      {/* Chart container */}
      <div ref={containerRef} className="flex-1 min-h-0" />
    </div>
  );
};

export default CandlestickChart;
