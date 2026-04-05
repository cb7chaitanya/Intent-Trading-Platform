import React, { useEffect, useState, useCallback, useRef } from "react";
import {
  Shield,
  Users,
  TrendingUp,
  Zap,
  Gavel,
  AlertTriangle,
  Database,
  Radio,
  Activity,
  Wifi,
  RefreshCw,
  ChevronDown,
  ChevronUp,
  Clock,
  XCircle,
} from "lucide-react";
import {
  getMetricsRaw,
  getHealthReady,
  getTopSolvers,
  getAdminStats,
  getAdminRecentIntents,
  getAdminRecentFills,
  getAdminFailedJobs,
  getAdminSolverPerformance,
} from "@/lib/api";
import { useAuth } from "@/contexts/AuthProvider";
import { useRouter } from "next/router";

// ── Types ──────────────────────────────────────────────────

interface StatsData {
  total_users: number;
  total_volume: number;
  active_intents: number;
  auctions_running: number;
  failed_settlements: number;
  websocket_connections: number;
}

interface PrometheusMetrics {
  apiLatencyP50: number | null;
  apiLatencyP99: number | null;
  dbConnections: number | null;
  redisStreamLag: number | null;
  cacheHitRate: number | null;
  totalRequests: number | null;
}

interface HealthStatus {
  status: string;
  services?: {
    db?: string;
    redis?: string;
    engine?: string;
  };
}

interface RecentIntent {
  id: string;
  user_id: string;
  token_in: string;
  token_out: string;
  amount_in: number;
  min_amount_out: number;
  status: string;
  order_type?: string;
  created_at: number;
}

interface RecentFill {
  id: string;
  intent_id: string;
  solver_id: string;
  price: number;
  qty: number;
  filled_qty: number;
  settled: boolean;
  timestamp: number;
}

interface SolverPerf {
  id: string;
  name: string;
  successful_trades: number;
  failed_trades: number;
  total_volume: number;
  reputation_score: number;
  avg_fill_time_ms?: number;
  win_rate: number;
}

interface FailedJob {
  id: string;
  job_type: string;
  reference_id: string;
  error: string;
  retry_count: number;
  permanently_failed: boolean;
  next_retry_at?: string;
  created_at: string;
}

// ── Prometheus Parser ──────────────────────────────────────

function parsePrometheusMetric(raw: string, name: string): number | null {
  const regex = new RegExp(`^${name}(?:\\{[^}]*\\})?\\s+([\\d.eE+-]+)`, "m");
  const match = raw.match(regex);
  return match ? parseFloat(match[1]) : null;
}

function parsePrometheusSum(raw: string, name: string): number {
  const regex = new RegExp(`^${name}(?:\\{[^}]*\\})?\\s+([\\d.eE+-]+)`, "gm");
  let total = 0;
  let match;
  while ((match = regex.exec(raw)) !== null) {
    total += parseFloat(match[1]);
  }
  return total;
}

function parsePrometheus(raw: string): PrometheusMetrics {
  return {
    apiLatencyP50: parsePrometheusMetric(raw, "api_request_duration_seconds_sum")
      ? Math.round(
          ((parsePrometheusSum(raw, "api_request_duration_seconds_sum") /
            Math.max(parsePrometheusSum(raw, "api_request_duration_seconds_count"), 1)) *
            1000)
        )
      : null,
    apiLatencyP99: null,
    dbConnections: parsePrometheusMetric(raw, "db_queries_total"),
    redisStreamLag: parsePrometheusMetric(raw, "cache_misses_total"),
    cacheHitRate: (() => {
      const hits = parsePrometheusSum(raw, "cache_hits_total");
      const misses = parsePrometheusSum(raw, "cache_misses_total");
      const total = hits + misses;
      return total > 0 ? Math.round((hits / total) * 100) : null;
    })(),
    totalRequests: parsePrometheusSum(raw, "api_requests_total"),
  };
}

// ── Stat Card ──────────────────────────────────────────────

interface StatCardProps {
  label: string;
  value: string | number;
  icon: React.ReactNode;
  color?: string;
  sub?: string;
}

const StatCard: React.FC<StatCardProps> = ({
  label,
  value,
  icon,
  color = "brand",
  sub,
}) => {
  const colorMap: Record<string, string> = {
    brand: "bg-brand-600/10 text-brand-400",
    green: "bg-green-500/10 text-up",
    red: "bg-red-500/10 text-down",
    yellow: "bg-yellow-500/10 text-yellow-500",
    purple: "bg-purple-500/10 text-purple-400",
    blue: "bg-blue-500/10 text-blue-400",
  };

  return (
    <div className="card flex items-center gap-3">
      <div
        className={`h-10 w-10 rounded-xl flex items-center justify-center shrink-0 ${
          colorMap[color] || colorMap.brand
        }`}
      >
        {icon}
      </div>
      <div className="min-w-0">
        <p className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider truncate">
          {label}
        </p>
        <p className="text-xl font-bold font-mono leading-tight">{value}</p>
        {sub && (
          <p className="text-[10px] text-[var(--text-muted)] truncate">{sub}</p>
        )}
      </div>
    </div>
  );
};

// ── Status Dot ─────────────────────────────────────────────

const StatusDot: React.FC<{ ok: boolean; label: string }> = ({
  ok,
  label,
}) => (
  <div className="flex items-center gap-1.5">
    <span
      className={`h-2 w-2 rounded-full ${ok ? "bg-up" : "bg-down"} ${
        ok ? "animate-pulse" : ""
      }`}
    />
    <span className="text-xs">{label}</span>
  </div>
);

// ── Intent Status Colors ───────────────────────────────────

const STATUS_COLORS: Record<string, string> = {
  Open: "badge-info",
  Bidding: "badge-warning",
  Matched: "badge-success",
  Executing: "bg-purple-500/10 text-purple-400",
  Completed: "badge-success",
  Failed: "badge-danger",
  Cancelled: "bg-surface-3 text-[var(--text-muted)]",
  Expired: "bg-surface-3 text-[var(--text-muted)]",
  PartiallyFilled: "badge-warning",
};

// ── Collapsible Table Section ──────────────────────────────

const TableSection: React.FC<{
  title: string;
  count?: number;
  children: React.ReactNode;
  defaultOpen?: boolean;
}> = ({ title, count, children, defaultOpen = true }) => {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="card !p-0 overflow-hidden">
      <button
        onClick={() => setOpen(!open)}
        className="w-full flex items-center justify-between px-4 py-3 hover:bg-surface-2 transition-colors"
      >
        <div className="flex items-center gap-2">
          <h3 className="text-sm font-semibold">{title}</h3>
          {count != null && (
            <span className="badge badge-info text-[10px]">{count}</span>
          )}
        </div>
        {open ? (
          <ChevronUp size={14} className="text-[var(--text-muted)]" />
        ) : (
          <ChevronDown size={14} className="text-[var(--text-muted)]" />
        )}
      </button>
      {open && <div className="border-t">{children}</div>}
    </div>
  );
};

// ── Main Page ───────────────────────────────────���──────────

export default function AdminDashboard() {
  const { user, hasRole } = useAuth();
  const router = useRouter();

  const [stats, setStats] = useState<StatsData | null>(null);
  const [prom, setProm] = useState<PrometheusMetrics | null>(null);
  const [health, setHealth] = useState<HealthStatus | null>(null);
  const [recentIntents, setRecentIntents] = useState<RecentIntent[]>([]);
  const [recentFills, setRecentFills] = useState<RecentFill[]>([]);
  const [solverPerf, setSolverPerf] = useState<SolverPerf[]>([]);
  const [failedJobs, setFailedJobs] = useState<FailedJob[]>([]);
  const [refreshing, setRefreshing] = useState(false);
  const [lastRefresh, setLastRefresh] = useState<Date | null>(null);
  const [autoRefresh, setAutoRefresh] = useState(true);
  const timerRef = useRef<ReturnType<typeof setInterval>>();

  const fetchAll = useCallback(async () => {
    setRefreshing(true);

    // Fetch everything in parallel; each call is best-effort
    const results = await Promise.allSettled([
      getAdminStats(),
      getMetricsRaw(),
      getHealthReady(),
      getAdminRecentIntents(20),
      getAdminRecentFills(20),
      getAdminSolverPerformance().catch(() => getTopSolvers(20)),
      getAdminFailedJobs(20),
    ]);

    if (results[0].status === "fulfilled") setStats(results[0].value);
    if (results[1].status === "fulfilled")
      setProm(parsePrometheus(results[1].value));
    if (results[2].status === "fulfilled") setHealth(results[2].value);
    if (results[3].status === "fulfilled")
      setRecentIntents(results[3].value || []);
    if (results[4].status === "fulfilled")
      setRecentFills(results[4].value || []);
    if (results[5].status === "fulfilled") {
      const solvers = results[5].value || [];
      setSolverPerf(
        solvers.map((s: any) => ({
          ...s,
          win_rate:
            s.win_rate ??
            (s.successful_trades + s.failed_trades > 0
              ? (s.successful_trades /
                  (s.successful_trades + s.failed_trades)) *
                100
              : 0),
        }))
      );
    }
    if (results[6].status === "fulfilled")
      setFailedJobs(results[6].value || []);

    setLastRefresh(new Date());
    setRefreshing(false);
  }, []);

  useEffect(() => {
    fetchAll();
  }, [fetchAll]);

  // Auto-refresh every 10s
  useEffect(() => {
    if (!autoRefresh) {
      clearInterval(timerRef.current);
      return;
    }
    timerRef.current = setInterval(fetchAll, 10000);
    return () => clearInterval(timerRef.current);
  }, [autoRefresh, fetchAll]);

  const dbOk = health?.services?.db === "ok" || health?.services?.db === "up";
  const redisOk =
    health?.services?.redis === "ok" || health?.services?.redis === "up";
  const engineOk =
    health?.services?.engine === "ok" || health?.services?.engine === "up";

  return (
    <div className="space-y-6 max-w-[1400px] mx-auto animate-fade-in">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
        <div className="flex items-center gap-3">
          <div className="h-10 w-10 rounded-xl bg-brand-600/10 flex items-center justify-center">
            <Shield size={20} className="text-brand-400" />
          </div>
          <div>
            <h1 className="text-2xl font-bold">Admin Dashboard</h1>
            <p className="text-xs text-[var(--text-muted)]">
              Platform overview and monitoring
            </p>
          </div>
        </div>

        <div className="flex items-center gap-3">
          {/* Service health dots */}
          <div className="hidden sm:flex items-center gap-3 border-r pr-3">
            <StatusDot ok={dbOk} label="DB" />
            <StatusDot ok={redisOk} label="Redis" />
            <StatusDot ok={engineOk} label="Engine" />
          </div>

          {/* Auto-refresh toggle */}
          <label className="flex items-center gap-1.5 text-xs text-[var(--text-muted)] cursor-pointer">
            <input
              type="checkbox"
              checked={autoRefresh}
              onChange={(e) => setAutoRefresh(e.target.checked)}
              className="rounded"
            />
            Auto (10s)
          </label>

          {/* Manual refresh */}
          <button
            onClick={fetchAll}
            disabled={refreshing}
            className="btn-ghost !p-2"
            aria-label="Refresh"
          >
            <RefreshCw size={16} className={refreshing ? "animate-spin" : ""} />
          </button>

          {lastRefresh && (
            <span className="text-[10px] text-[var(--text-muted)]">
              {lastRefresh.toLocaleTimeString()}
            </span>
          )}
        </div>
      </div>

      {/* ── Stat Cards Grid ─────────────────────────────── */}
      <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-5 gap-3">
        <StatCard
          label="Total Users"
          value={stats?.total_users?.toLocaleString() ?? "—"}
          icon={<Users size={18} />}
          color="blue"
        />
        <StatCard
          label="Total Volume"
          value={
            stats?.total_volume != null
              ? stats.total_volume >= 1_000_000
                ? `${(stats.total_volume / 1_000_000).toFixed(1)}M`
                : stats.total_volume.toLocaleString()
              : "—"
          }
          icon={<TrendingUp size={18} />}
          color="green"
        />
        <StatCard
          label="Active Intents"
          value={stats?.active_intents?.toLocaleString() ?? "—"}
          icon={<Zap size={18} />}
          color="brand"
        />
        <StatCard
          label="Auctions Running"
          value={stats?.auctions_running?.toLocaleString() ?? "—"}
          icon={<Gavel size={18} />}
          color="purple"
        />
        <StatCard
          label="Failed Settlements"
          value={stats?.failed_settlements?.toLocaleString() ?? "—"}
          icon={<AlertTriangle size={18} />}
          color={(stats?.failed_settlements ?? 0) > 0 ? "red" : "green"}
        />
      </div>

      {/* ── Infrastructure Row ──────────────────────────── */}
      <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-5 gap-3">
        <StatCard
          label="Redis Stream Lag"
          value={prom?.redisStreamLag?.toLocaleString() ?? "—"}
          icon={<Radio size={18} />}
          color="yellow"
          sub="cache misses"
        />
        <StatCard
          label="DB Queries"
          value={prom?.dbConnections?.toLocaleString() ?? "—"}
          icon={<Database size={18} />}
          color="blue"
          sub={dbOk ? "healthy" : "degraded"}
        />
        <StatCard
          label="API Latency"
          value={prom?.apiLatencyP50 != null ? `${prom.apiLatencyP50}ms` : "—"}
          icon={<Activity size={18} />}
          color={
            prom?.apiLatencyP50 != null && prom.apiLatencyP50 > 500
              ? "red"
              : "green"
          }
          sub="avg response"
        />
        <StatCard
          label="WebSocket Conns"
          value={stats?.websocket_connections?.toLocaleString() ?? "—"}
          icon={<Wifi size={18} />}
          color="purple"
        />
        <StatCard
          label="Cache Hit Rate"
          value={prom?.cacheHitRate != null ? `${prom.cacheHitRate}%` : "—"}
          icon={<Zap size={18} />}
          color={
            prom?.cacheHitRate != null && prom.cacheHitRate < 50
              ? "yellow"
              : "green"
          }
          sub={`${prom?.totalRequests?.toLocaleString() ?? "—"} total reqs`}
        />
      </div>

      {/* ── Solver Leaderboard ──────────────────────────── */}
      <TableSection title="Solver Performance" count={solverPerf.length}>
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-[10px] text-[var(--text-muted)] uppercase tracking-wider border-b">
                <th className="px-4 py-2.5 text-left font-medium">#</th>
                <th className="px-4 py-2.5 text-left font-medium">Solver</th>
                <th className="px-4 py-2.5 text-right font-medium">Wins</th>
                <th className="px-4 py-2.5 text-right font-medium">Losses</th>
                <th className="px-4 py-2.5 text-right font-medium">
                  Win Rate
                </th>
                <th className="px-4 py-2.5 text-right font-medium">Volume</th>
                <th className="px-4 py-2.5 text-right font-medium">Score</th>
              </tr>
            </thead>
            <tbody>
              {solverPerf.map((s, i) => (
                <tr
                  key={s.id}
                  className="border-b border-[var(--border)] hover:bg-surface-2 transition-colors"
                >
                  <td className="px-4 py-2 text-[var(--text-muted)] font-mono text-xs">
                    {i + 1}
                  </td>
                  <td className="px-4 py-2 font-medium">{s.name}</td>
                  <td className="px-4 py-2 text-right font-mono text-up">
                    {s.successful_trades}
                  </td>
                  <td className="px-4 py-2 text-right font-mono text-down">
                    {s.failed_trades}
                  </td>
                  <td className="px-4 py-2 text-right">
                    <div className="flex items-center justify-end gap-2">
                      <div className="w-16 h-1.5 rounded-full bg-surface-2 overflow-hidden">
                        <div
                          className="h-full rounded-full bg-up"
                          style={{
                            width: `${Math.min(s.win_rate, 100)}%`,
                          }}
                        />
                      </div>
                      <span className="font-mono text-xs w-10 text-right">
                        {s.win_rate.toFixed(0)}%
                      </span>
                    </div>
                  </td>
                  <td className="px-4 py-2 text-right font-mono text-xs text-[var(--text-muted)]">
                    {s.total_volume.toLocaleString()}
                  </td>
                  <td className="px-4 py-2 text-right font-mono font-semibold text-brand-400">
                    {s.reputation_score.toFixed(2)}
                  </td>
                </tr>
              ))}
              {solverPerf.length === 0 && (
                <tr>
                  <td
                    colSpan={7}
                    className="px-4 py-8 text-center text-[var(--text-muted)]"
                  >
                    No solver data
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </TableSection>

      {/* ── Tables Row ──────────────────────────────────── */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Recent Intents */}
        <TableSection
          title="Recent Intents"
          count={recentIntents.length}
        >
          <div className="overflow-x-auto">
            <table className="w-full text-[11px]">
              <thead>
                <tr className="text-[10px] text-[var(--text-muted)] uppercase tracking-wider border-b">
                  <th className="px-3 py-2 text-left font-medium">ID</th>
                  <th className="px-3 py-2 text-left font-medium">Pair</th>
                  <th className="px-3 py-2 text-left font-medium">Type</th>
                  <th className="px-3 py-2 text-right font-medium">Amount</th>
                  <th className="px-3 py-2 text-center font-medium">Status</th>
                  <th className="px-3 py-2 text-right font-medium">Time</th>
                </tr>
              </thead>
              <tbody>
                {recentIntents.map((intent) => (
                  <tr
                    key={intent.id}
                    className="border-b border-[var(--border)] hover:bg-surface-2 transition-colors"
                  >
                    <td className="px-3 py-2 font-mono text-[var(--text-muted)]">
                      {intent.id.slice(0, 8)}
                    </td>
                    <td className="px-3 py-2 font-medium">
                      {intent.token_in}/{intent.token_out}
                    </td>
                    <td className="px-3 py-2 text-[var(--text-muted)]">
                      {intent.order_type || "market"}
                    </td>
                    <td className="px-3 py-2 text-right font-mono">
                      {intent.amount_in.toLocaleString()}
                    </td>
                    <td className="px-3 py-2 text-center">
                      <span
                        className={`badge ${
                          STATUS_COLORS[intent.status] || "badge-info"
                        }`}
                      >
                        {intent.status}
                      </span>
                    </td>
                    <td className="px-3 py-2 text-right text-[var(--text-muted)] font-mono">
                      {new Date(intent.created_at * 1000).toLocaleTimeString(
                        [],
                        { hour: "2-digit", minute: "2-digit", second: "2-digit" }
                      )}
                    </td>
                  </tr>
                ))}
                {recentIntents.length === 0 && (
                  <tr>
                    <td
                      colSpan={6}
                      className="px-3 py-6 text-center text-[var(--text-muted)]"
                    >
                      No recent intents
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        </TableSection>

        {/* Recent Fills */}
        <TableSection title="Recent Fills" count={recentFills.length}>
          <div className="overflow-x-auto">
            <table className="w-full text-[11px]">
              <thead>
                <tr className="text-[10px] text-[var(--text-muted)] uppercase tracking-wider border-b">
                  <th className="px-3 py-2 text-left font-medium">Fill ID</th>
                  <th className="px-3 py-2 text-left font-medium">Solver</th>
                  <th className="px-3 py-2 text-right font-medium">Price</th>
                  <th className="px-3 py-2 text-right font-medium">Qty</th>
                  <th className="px-3 py-2 text-right font-medium">Filled</th>
                  <th className="px-3 py-2 text-center font-medium">
                    Settled
                  </th>
                </tr>
              </thead>
              <tbody>
                {recentFills.map((fill) => (
                  <tr
                    key={fill.id}
                    className="border-b border-[var(--border)] hover:bg-surface-2 transition-colors"
                  >
                    <td className="px-3 py-2 font-mono text-[var(--text-muted)]">
                      {fill.id.slice(0, 8)}
                    </td>
                    <td className="px-3 py-2 font-medium">
                      {fill.solver_id.length > 12
                        ? `${fill.solver_id.slice(0, 12)}...`
                        : fill.solver_id}
                    </td>
                    <td className="px-3 py-2 text-right font-mono text-up">
                      {fill.price.toLocaleString()}
                    </td>
                    <td className="px-3 py-2 text-right font-mono">
                      {fill.qty.toLocaleString()}
                    </td>
                    <td className="px-3 py-2 text-right font-mono">
                      {fill.filled_qty.toLocaleString()}
                    </td>
                    <td className="px-3 py-2 text-center">
                      <span
                        className={`badge ${
                          fill.settled ? "badge-success" : "badge-warning"
                        }`}
                      >
                        {fill.settled ? "Yes" : "Pending"}
                      </span>
                    </td>
                  </tr>
                ))}
                {recentFills.length === 0 && (
                  <tr>
                    <td
                      colSpan={6}
                      className="px-3 py-6 text-center text-[var(--text-muted)]"
                    >
                      No recent fills
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        </TableSection>
      </div>

      {/* ── Failed Jobs ─────────────────────────────────── */}
      <TableSection
        title="Failed Jobs"
        count={failedJobs.length}
        defaultOpen={failedJobs.length > 0}
      >
        <div className="overflow-x-auto">
          <table className="w-full text-[11px]">
            <thead>
              <tr className="text-[10px] text-[var(--text-muted)] uppercase tracking-wider border-b">
                <th className="px-4 py-2.5 text-left font-medium">Type</th>
                <th className="px-4 py-2.5 text-left font-medium">
                  Reference
                </th>
                <th className="px-4 py-2.5 text-left font-medium">Error</th>
                <th className="px-4 py-2.5 text-right font-medium">
                  Retries
                </th>
                <th className="px-4 py-2.5 text-center font-medium">
                  Status
                </th>
                <th className="px-4 py-2.5 text-right font-medium">
                  Next Retry
                </th>
                <th className="px-4 py-2.5 text-right font-medium">
                  Created
                </th>
              </tr>
            </thead>
            <tbody>
              {failedJobs.map((job) => (
                <tr
                  key={job.id}
                  className={`border-b border-[var(--border)] hover:bg-surface-2 transition-colors ${
                    job.permanently_failed ? "opacity-60" : ""
                  }`}
                >
                  <td className="px-4 py-2.5">
                    <div className="flex items-center gap-1.5">
                      {job.permanently_failed ? (
                        <XCircle size={12} className="text-down shrink-0" />
                      ) : (
                        <Clock
                          size={12}
                          className="text-yellow-500 shrink-0"
                        />
                      )}
                      <span className="font-medium">{job.job_type}</span>
                    </div>
                  </td>
                  <td className="px-4 py-2.5 font-mono text-[var(--text-muted)]">
                    {job.reference_id.slice(0, 12)}...
                  </td>
                  <td className="px-4 py-2.5 max-w-xs">
                    <p className="text-down truncate" title={job.error}>
                      {job.error}
                    </p>
                  </td>
                  <td className="px-4 py-2.5 text-right font-mono">
                    {job.retry_count}
                  </td>
                  <td className="px-4 py-2.5 text-center">
                    <span
                      className={`badge ${
                        job.permanently_failed
                          ? "badge-danger"
                          : "badge-warning"
                      }`}
                    >
                      {job.permanently_failed ? "Dead" : "Retrying"}
                    </span>
                  </td>
                  <td className="px-4 py-2.5 text-right text-[var(--text-muted)] font-mono">
                    {job.next_retry_at
                      ? new Date(job.next_retry_at).toLocaleTimeString([], {
                          hour: "2-digit",
                          minute: "2-digit",
                        })
                      : "—"}
                  </td>
                  <td className="px-4 py-2.5 text-right text-[var(--text-muted)]">
                    {new Date(job.created_at).toLocaleString([], {
                      month: "short",
                      day: "numeric",
                      hour: "2-digit",
                      minute: "2-digit",
                    })}
                  </td>
                </tr>
              ))}
              {failedJobs.length === 0 && (
                <tr>
                  <td
                    colSpan={7}
                    className="px-4 py-8 text-center text-[var(--text-muted)]"
                  >
                    <div className="flex flex-col items-center gap-1">
                      <span className="text-up text-lg">&#10003;</span>
                      <span>No failed jobs</span>
                    </div>
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </TableSection>
    </div>
  );
}
