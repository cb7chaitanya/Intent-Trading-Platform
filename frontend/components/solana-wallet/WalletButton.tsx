import React from "react";
import { useWallet } from "@solana/wallet-adapter-react";
import { useWalletModal } from "@solana/wallet-adapter-react-ui";
import { Wallet, LogOut, Copy, Check } from "lucide-react";

const WalletButton: React.FC = () => {
  const { publicKey, connected, disconnect, connecting } = useWallet();
  const { setVisible } = useWalletModal();
  const [copied, setCopied] = React.useState(false);

  const truncatedAddress = publicKey
    ? `${publicKey.toBase58().slice(0, 4)}...${publicKey.toBase58().slice(-4)}`
    : "";

  const handleCopy = async () => {
    if (!publicKey) return;
    await navigator.clipboard.writeText(publicKey.toBase58());
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  if (!connected) {
    return (
      <button
        onClick={() => setVisible(true)}
        disabled={connecting}
        className="flex items-center gap-2 rounded-lg bg-purple-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-purple-700 transition-colors disabled:opacity-50"
      >
        <Wallet size={14} />
        {connecting ? "Connecting..." : "Connect Wallet"}
      </button>
    );
  }

  return (
    <div className="flex items-center gap-1.5">
      {/* Address badge */}
      <button
        onClick={handleCopy}
        className="flex items-center gap-1.5 rounded-lg bg-surface-2 px-2.5 py-1.5 text-xs font-mono hover:bg-surface-3 transition-colors"
        title={publicKey?.toBase58()}
      >
        <span className="h-2 w-2 rounded-full bg-purple-400 animate-pulse" />
        <span>{truncatedAddress}</span>
        {copied ? (
          <Check size={12} className="text-up" />
        ) : (
          <Copy size={12} className="text-[var(--text-muted)]" />
        )}
      </button>

      {/* Disconnect */}
      <button
        onClick={disconnect}
        className="btn-ghost !p-1.5 text-[var(--text-muted)] hover:text-down"
        title="Disconnect wallet"
      >
        <LogOut size={14} />
      </button>
    </div>
  );
};

export default WalletButton;
