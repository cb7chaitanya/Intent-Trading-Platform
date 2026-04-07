import React, { useEffect, useState, useCallback } from "react";
import {
  CheckCircle,
  Circle,
  Clock,
  AlertTriangle,
  RefreshCw,
  XCircle,
  ExternalLink,
  Loader2,
} from "lucide-react";
import { getCrossChainLegs } from "@/lib/api";
import { useWebSocket } from "@/contexts/WebSocketProvider";

// ── Types ───────────────────────────────────────────────

interface CrossChainLeg {
  id: string;
  intent_id: string;
  fill_id: string;
  leg_index: number; // 0 = source, 1 = dest
  chain: string;
  from_address: string;
  to_address: string;
  token_mint: string | null;
  amount: number;
  tx_hash: string | null;
  status: LegStatus;
  error: string | null;
  timeout_at: string;
  created_at: string;
  confirmed_at: string | null;
}

type LegStatus =
  | "pending"
  | "escrowed"
  | "executing"
  | "confirmed"
  | "failed"
  | "refunded";

// ── Step definitions ────────────────────────────────────

interface Step {
  label: string;
  description: string;
  status: "upcoming" | "active" | "done" | "failed" | "refunded";
}

function deriveSteps(
  source: CrossChainLeg | null,
  dest: CrossChainLeg | null
): Step[] {
  const ss = source?.status || "pending";
  const ds = dest?.status || "pending";

  const steps: Step[] = [
    {
      label: "Submitted",
      description: "Intent submitted to the network",
      status:
        ss === "pending" && !source?.tx_hash ? "active" : "done",
    },
    {
      label: "Locked",
      description: `Funds escrowed on ${source?.chain || "source"} chain`,
      status:
        ss === "pending"
          ? "upcoming"
          : ss === "escrowed"
          ? "active"
          : ss === "failed"
          ? "failed"
          : ss === "refunded"
          ? "refunded"
          : "done",
    },
    {
      label: "Verified",
      description: "Bridge message verified by guardians / DVNs",
      status:
        ss === "confirmed" && ds === "pending"
          ? "active"
          : ss === "confirmed" &&
            (ds === "executing" || ds === "confirmed")
          ? "done"
          : ["pending", "escrowed"].includes(ss)
          ? "upcoming"
          : ss === "failed" || ss === "refunded"
          ? ss === "failed"
            ? "failed"
            : "refunded"
          : "upcoming",
    },
    {
      label: "Released",
      description: `Funds delivered on ${dest?.chain || "destination"} chain`,
      status:
        ds === "confirmed"
          ? "done"
          : ds === "executing"
          ? "active"
          : ds === "failed"
          ? "failed"
          : ds === "refunded"
          ? "refunded"
          : "upcoming",
    },
  ];

  return steps;
}

// ── Step icon ───────────────────────────────────────────

function StepIcon({ status }: { status: Step["status"] }) {
  switch (status) {
    case "done":
      return <CheckCircle size={18} className="text-up" />;
    case "active":
      return <Loader2 size={18} className="text-brand-400 animate-spin" />;
    case "failed":
      return <XCircle size={18} className="text-down" />;
    case "refunded":
      return <AlertTriangle size={18} className="text-yellow-500" />;
    default:
      return <Circle size={18} className="text-[var(--text-muted)]" />;
  }
}

// ── Explorer links ──────────────────────────────────────

function explorerUrl(chain: string, txHash: string): string | null {
  const base: Record<string, string> = {
    ethereum: "https://etherscan.io/tx/",
    polygon: "https://polygonscan.com/tx/",
    arbitrum: "https://arbiscan.io/tx/",
    base: "https://basescan.org/tx/",
    solana: "https://solscan.io/tx/",
  };
  const prefix = base[chain];
  return prefix ? `${prefix}${txHash}` : null;
}

// ── Main component ──────────────────────────────────────

interface SettlementTrackerProps {
  intentId: string;
  /** Compact inline mode vs expanded card mode */
  compact?: boolean;
}

const SettlementTracker: React.FC<SettlementTrackerProps> = ({
  intentId,
  compact = false,
}) => {
  const [legs, setLegs] = useState<CrossChainLeg[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const { addListener } = useWebSocket();

  const source = legs.find((l) => l.leg_index === 0) || null;
  const dest = legs.find((l) => l.leg_index === 1) || null;
  const steps = deriveSteps(source, dest);

  const isTerminal =
    (source?.status === "confirmed" && dest?.status === "confirmed") ||
    source?.status === "refunded" ||
    dest?.status === "refunded" ||
    source?.status === "failed";

  const fetchLegs = useCallback(async () => {
    try {
      const data = await getCrossChainLegs(intentId);
      setLegs(Array.isArray(data) ? data : []);
      setError(null);
    } catch (e: any) {
      setError(e?.message || "Failed to load settlement");
    } finally {
      setLoading(false);
    }
  }, [intentId]);

  // Initial fetch + polling
  useEffect(() => {
    fetchLegs();

    if (!isTerminal) {
      const interval = setInterval(fetchLegs, 5_000);
      return () => clearInterval(interval);
    }
  }, [fetchLegs, isTerminal]);

  // Listen for WebSocket updates
  useEffect(() => {
    return addListener("CrossChainUpdate", (data: any) => {
      if (data?.intent_id === intentId) {
        fetchLegs();
      }
    });
  }, [addListener, intentId, fetchLegs]);

  if (loading) {
    return (
      <div className="flex items-center gap-2 text-xs text-[var(--text-muted)] py-2">
        <Loader2 size={14} className="animate-spin" />
        Loading settlement...
      </div>
    );
  }

  if (error || legs.length === 0) {
    return null; // Not a cross-chain intent
  }

  // Compact mode: single-line status
  if (compact) {
    const activeStep = steps.find((s) => s.status === "active");
    const failedStep = steps.find(
      (s) => s.status === "failed" || s.status === "refunded"
    );
    const display = failedStep || activeStep || steps[steps.length - 1];

    return (
      <div className="flex items-center gap-1.5">
        <StepIcon status={display.status} />
        <span className="text-[11px] font-medium">{display.label}</span>
        {source?.chain && dest?.chain && (
          <span className="text-[10px] text-[var(--text-muted)]">
            {source.chain} → {dest.chain}
          </span>
        )}
      </div>
    );
  }

  // Expanded card mode
  return (
    <div className="rounded-xl border bg-surface-1 overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b bg-surface-2">
        <div className="flex items-center gap-2">
          <span className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider">
            Cross-Chain Settlement
          </span>
          {source?.chain && dest?.chain && (
            <span className="badge badge-info text-[10px]">
              {source.chain} → {dest.chain}
            </span>
          )}
        </div>
        <button
          onClick={fetchLegs}
          className="btn-ghost !p-1"
          title="Refresh"
        >
          <RefreshCw size={12} />
        </button>
      </div>

      {/* Step progress */}
      <div className="px-3 py-3">
        <div className="space-y-0">
          {steps.map((step, i) => (
            <div key={step.label} className="flex items-start gap-3">
              {/* Vertical line + icon */}
              <div className="flex flex-col items-center">
                <StepIcon status={step.status} />
                {i < steps.length - 1 && (
                  <div
                    className={`w-px h-6 mt-1 ${
                      step.status === "done"
                        ? "bg-up"
                        : step.status === "failed"
                        ? "bg-down"
                        : "bg-[var(--border)]"
                    }`}
                  />
                )}
              </div>

              {/* Label + description */}
              <div className="pb-4">
                <div
                  className={`text-xs font-medium ${
                    step.status === "active"
                      ? "text-brand-400"
                      : step.status === "done"
                      ? "text-up"
                      : step.status === "failed"
                      ? "text-down"
                      : step.status === "refunded"
                      ? "text-yellow-500"
                      : "text-[var(--text-muted)]"
                  }`}
                >
                  {step.label}
                </div>
                <div className="text-[11px] text-[var(--text-muted)]">
                  {step.description}
                </div>
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* Transaction hashes */}
      {(source?.tx_hash || dest?.tx_hash) && (
        <div className="px-3 pb-3 space-y-1.5 border-t pt-2">
          {source?.tx_hash && (
            <TxLink chain={source.chain} hash={source.tx_hash} label="Source tx" />
          )}
          {dest?.tx_hash && (
            <TxLink chain={dest.chain} hash={dest.tx_hash} label="Dest tx" />
          )}
        </div>
      )}

      {/* Failure / refund state */}
      {(source?.status === "failed" || dest?.status === "failed") && (
        <div className="px-3 pb-3">
          <div className="rounded-lg bg-down/10 px-3 py-2 text-[11px] text-down">
            <div className="font-medium mb-0.5">Settlement failed</div>
            <div>{source?.error || dest?.error || "Unknown error"}</div>
          </div>
        </div>
      )}

      {(source?.status === "refunded" || dest?.status === "refunded") && (
        <div className="px-3 pb-3">
          <div className="rounded-lg bg-yellow-500/10 px-3 py-2 text-[11px] text-yellow-500">
            <div className="font-medium mb-0.5">Funds refunded</div>
            <div>
              The bridge transfer timed out. Your funds have been returned to
              your source chain wallet.
            </div>
          </div>
        </div>
      )}

      {/* Timeout indicator */}
      {!isTerminal && source?.timeout_at && (
        <div className="px-3 pb-3">
          <TimeoutBar timeoutAt={source.timeout_at} />
        </div>
      )}
    </div>
  );
};

// ── Sub-components ──────────────────────────────────────

function TxLink({
  chain,
  hash,
  label,
}: {
  chain: string;
  hash: string;
  label: string;
}) {
  const url = explorerUrl(chain, hash);
  return (
    <div className="flex items-center justify-between">
      <span className="text-[10px] text-[var(--text-muted)]">{label}</span>
      <div className="flex items-center gap-1">
        <span className="text-[10px] font-mono text-[var(--text-secondary)]">
          {hash.slice(0, 10)}...{hash.slice(-6)}
        </span>
        {url && (
          <a
            href={url}
            target="_blank"
            rel="noopener noreferrer"
            className="text-brand-400 hover:text-brand-300"
          >
            <ExternalLink size={10} />
          </a>
        )}
      </div>
    </div>
  );
}

function TimeoutBar({ timeoutAt }: { timeoutAt: string }) {
  const [pct, setPct] = useState(0);

  useEffect(() => {
    const update = () => {
      const now = Date.now();
      const timeout = new Date(timeoutAt).getTime();
      // Assume 10 min total window
      const total = 600_000;
      const start = timeout - total;
      const elapsed = now - start;
      setPct(Math.min(100, Math.max(0, (elapsed / total) * 100)));
    };

    update();
    const interval = setInterval(update, 1000);
    return () => clearInterval(interval);
  }, [timeoutAt]);

  const remaining = Math.max(
    0,
    Math.floor((new Date(timeoutAt).getTime() - Date.now()) / 1000)
  );
  const mins = Math.floor(remaining / 60);
  const secs = remaining % 60;

  return (
    <div>
      <div className="flex items-center justify-between mb-1">
        <span className="text-[10px] text-[var(--text-muted)] flex items-center gap-1">
          <Clock size={10} />
          Settlement timeout
        </span>
        <span
          className={`text-[10px] font-mono ${
            remaining < 60 ? "text-down" : "text-[var(--text-secondary)]"
          }`}
        >
          {mins}:{String(secs).padStart(2, "0")} remaining
        </span>
      </div>
      <div className="h-1 rounded-full bg-surface-2 overflow-hidden">
        <div
          className={`h-full rounded-full transition-all duration-1000 ${
            pct > 80 ? "bg-down" : pct > 50 ? "bg-yellow-500" : "bg-brand-500"
          }`}
          style={{ width: `${pct}%` }}
        />
      </div>
    </div>
  );
}

export default SettlementTracker;
