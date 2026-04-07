import React from "react";
import { ArrowRight } from "lucide-react";

export interface Chain {
  id: string;
  name: string;
  icon: string; // emoji or short label
  color: string; // tailwind text color class
}

export const SUPPORTED_CHAINS: Chain[] = [
  { id: "ethereum", name: "Ethereum", icon: "E", color: "text-blue-400" },
  { id: "arbitrum", name: "Arbitrum", icon: "A", color: "text-sky-400" },
  { id: "base", name: "Base", icon: "B", color: "text-blue-300" },
  { id: "polygon", name: "Polygon", icon: "P", color: "text-purple-400" },
  { id: "solana", name: "Solana", icon: "S", color: "text-green-400" },
];

interface ChainSelectorProps {
  sourceChain: string;
  destChain: string;
  onSourceChange: (chain: string) => void;
  onDestChange: (chain: string) => void;
  onSwap: () => void;
}

const ChainSelector: React.FC<ChainSelectorProps> = ({
  sourceChain,
  destChain,
  onSourceChange,
  onDestChange,
  onSwap,
}) => {
  return (
    <div className="flex items-center gap-2">
      {/* Source chain */}
      <div className="flex-1">
        <label className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider mb-1 block">
          From
        </label>
        <select
          value={sourceChain}
          onChange={(e) => onSourceChange(e.target.value)}
          className="input w-full text-sm font-medium"
        >
          {SUPPORTED_CHAINS.map((c) => (
            <option
              key={c.id}
              value={c.id}
              disabled={c.id === destChain}
            >
              {c.icon} {c.name}
            </option>
          ))}
        </select>
      </div>

      {/* Swap button */}
      <button
        type="button"
        onClick={onSwap}
        className="btn-ghost !p-2 rounded-lg mt-5 hover:bg-surface-2"
        title="Swap chains"
      >
        <ArrowRight size={16} className="text-[var(--text-muted)]" />
      </button>

      {/* Destination chain */}
      <div className="flex-1">
        <label className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider mb-1 block">
          To
        </label>
        <select
          value={destChain}
          onChange={(e) => onDestChange(e.target.value)}
          className="input w-full text-sm font-medium"
        >
          {SUPPORTED_CHAINS.map((c) => (
            <option
              key={c.id}
              value={c.id}
              disabled={c.id === sourceChain}
            >
              {c.icon} {c.name}
            </option>
          ))}
        </select>
      </div>
    </div>
  );
};

export default ChainSelector;
