import React, { useEffect, useState } from "react";
import { Clock, AlertTriangle, Shield, Zap } from "lucide-react";
import { getBridgeFeeEstimate, getBridgeRoutes } from "@/lib/api";

interface BridgeRoute {
  bridge: string;
  supported: boolean;
}

interface FeeEstimate {
  source_fee: number;
  dest_fee: number;
  protocol_fee: number;
  total_description: string;
}

interface BridgeTime {
  min_secs: number;
  typical_secs: number;
  max_secs: number;
}

interface RouteInfoProps {
  sourceChain: string;
  destChain: string;
  token: string;
  amount: number;
}

const BRIDGE_META: Record<string, { label: string; icon: typeof Shield }> = {
  wormhole: { label: "Wormhole", icon: Shield },
  layerzero: { label: "LayerZero", icon: Zap },
};

function formatTime(secs: number): string {
  if (secs < 60) return `~${secs}s`;
  if (secs < 3600) return `~${Math.ceil(secs / 60)}min`;
  return `~${(secs / 3600).toFixed(1)}h`;
}

const RouteInfo: React.FC<RouteInfoProps> = ({
  sourceChain,
  destChain,
  token,
  amount,
}) => {
  const [routes, setRoutes] = useState<BridgeRoute[]>([]);
  const [fee, setFee] = useState<FeeEstimate | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!sourceChain || !destChain || sourceChain === destChain) {
      setRoutes([]);
      setFee(null);
      return;
    }

    setLoading(true);

    Promise.all([
      getBridgeRoutes(sourceChain, destChain).catch(() => []),
      amount > 0
        ? getBridgeFeeEstimate({
            source_chain: sourceChain,
            dest_chain: destChain,
            token,
            amount,
          }).catch(() => null)
        : Promise.resolve(null),
    ])
      .then(([r, f]) => {
        setRoutes(Array.isArray(r) ? r : []);
        setFee(f);
      })
      .finally(() => setLoading(false));
  }, [sourceChain, destChain, token, amount]);

  if (sourceChain === destChain) return null;

  const availableRoutes = routes.filter((r) => r.supported);

  // Estimate bridge time based on source chain
  const estimatedTime: BridgeTime = (() => {
    const finality: Record<string, number> = {
      ethereum: 960,
      solana: 15,
      polygon: 256,
      arbitrum: 10,
      base: 10,
    };
    const sf = finality[sourceChain] || 120;
    return { min_secs: sf + 10, typical_secs: sf + 60, max_secs: sf + 600 };
  })();

  // Risk level based on route
  const isHighRisk = sourceChain === "ethereum" && estimatedTime.typical_secs > 600;
  const isNewRoute =
    (sourceChain === "base" || destChain === "base") &&
    availableRoutes.some((r) => r.bridge === "layerzero");

  return (
    <div className="rounded-lg border bg-surface-2 p-3 space-y-2.5">
      {/* Bridge routes */}
      <div>
        <div className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider mb-1.5">
          Bridge Route
        </div>
        {loading ? (
          <div className="text-xs text-[var(--text-muted)]">Loading routes...</div>
        ) : availableRoutes.length > 0 ? (
          <div className="flex gap-1.5">
            {availableRoutes.map((r) => {
              const meta = BRIDGE_META[r.bridge] || {
                label: r.bridge,
                icon: Shield,
              };
              const Icon = meta.icon;
              return (
                <div
                  key={r.bridge}
                  className="flex items-center gap-1 rounded-md bg-surface-1 px-2 py-1 text-xs font-medium"
                >
                  <Icon size={12} className="text-brand-400" />
                  {meta.label}
                </div>
              );
            })}
          </div>
        ) : (
          <div className="text-xs text-down">No bridge route available</div>
        )}
      </div>

      {/* Estimated time */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-1 text-xs text-[var(--text-secondary)]">
          <Clock size={12} />
          Est. time
        </div>
        <div className="text-xs font-mono font-medium">
          {formatTime(estimatedTime.typical_secs)}
          <span className="text-[var(--text-muted)] ml-1">
            ({formatTime(estimatedTime.min_secs)} - {formatTime(estimatedTime.max_secs)})
          </span>
        </div>
      </div>

      {/* Fees */}
      {fee && (
        <div className="flex items-center justify-between">
          <div className="text-xs text-[var(--text-secondary)]">Bridge fee</div>
          <div className="text-xs font-mono font-medium">
            {fee.total_description}
          </div>
        </div>
      )}

      {/* Risk warnings */}
      {(isHighRisk || isNewRoute) && (
        <div className="space-y-1.5 pt-1 border-t border-[var(--border)]">
          {isHighRisk && (
            <div className="flex items-start gap-1.5 text-[11px] text-yellow-500">
              <AlertTriangle size={12} className="shrink-0 mt-0.5" />
              <span>
                Ethereum source requires ~15 min for finality. Funds are locked
                until the bridge message is confirmed.
              </span>
            </div>
          )}
          {isNewRoute && (
            <div className="flex items-start gap-1.5 text-[11px] text-yellow-500">
              <AlertTriangle size={12} className="shrink-0 mt-0.5" />
              <span>
                Base routes via LayerZero are newer. Consider smaller amounts for
                initial transfers.
              </span>
            </div>
          )}
        </div>
      )}
    </div>
  );
};

export default RouteInfo;
