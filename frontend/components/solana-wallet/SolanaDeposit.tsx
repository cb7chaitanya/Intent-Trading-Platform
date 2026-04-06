import React, { useState } from "react";
import { useConnection, useWallet } from "@solana/wallet-adapter-react";
import { PublicKey } from "@solana/web3.js";
import { ArrowDownToLine, Loader2, AlertCircle } from "lucide-react";
import { sendDeposit } from "@/lib/solana";

interface SolanaDepositProps {
  mintAddress: string;
  vaultTokenAccount: string;
  decimals?: number;
  symbol?: string;
}

const SolanaDeposit: React.FC<SolanaDepositProps> = ({
  mintAddress,
  vaultTokenAccount,
  decimals = 6,
  symbol = "USDC",
}) => {
  const { connection } = useConnection();
  const { publicKey, signTransaction, connected } = useWallet();

  const [amount, setAmount] = useState("");
  const [loading, setLoading] = useState(false);
  const [status, setStatus] = useState<{
    type: "success" | "error";
    msg: string;
    sig?: string;
  } | null>(null);

  const amountNum = Number(amount) || 0;

  const handleDeposit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!publicKey || !signTransaction || amountNum <= 0) return;

    setLoading(true);
    setStatus(null);

    try {
      const mint = new PublicKey(mintAddress);
      const vault = new PublicKey(vaultTokenAccount);
      const rawAmount = BigInt(Math.floor(amountNum * 10 ** decimals));

      const sig = await sendDeposit({
        connection,
        wallet: publicKey,
        mint,
        vaultTokenAccount: vault,
        amount: rawAmount,
        signTransaction,
      });

      setStatus({
        type: "success",
        msg: `Deposited ${amountNum} ${symbol}`,
        sig,
      });
      setAmount("");
    } catch (err: any) {
      setStatus({
        type: "error",
        msg: err?.message || "Deposit failed",
      });
    } finally {
      setLoading(false);
    }
  };

  if (!connected) {
    return (
      <div className="card text-center py-6">
        <p className="text-sm text-[var(--text-muted)]">
          Connect your Solana wallet to deposit
        </p>
      </div>
    );
  }

  return (
    <form onSubmit={handleDeposit} className="card space-y-3">
      <h3 className="text-sm font-semibold">Deposit to Settlement Vault</h3>

      <div className="space-y-1">
        <label className="text-[10px] font-medium text-[var(--text-muted)] uppercase tracking-wider">
          Amount
        </label>
        <div className="relative">
          <input
            type="number"
            className="input font-mono !text-sm !pr-16"
            placeholder="0.00"
            value={amount}
            onChange={(e) => setAmount(e.target.value)}
            min={0}
            step={10 ** -decimals}
            required
          />
          <span className="absolute right-3 top-1/2 -translate-y-1/2 text-xs text-[var(--text-muted)]">
            {symbol}
          </span>
        </div>
      </div>

      <button
        type="submit"
        disabled={loading || amountNum <= 0}
        className="w-full py-2.5 text-xs font-semibold rounded-lg bg-purple-600 text-white hover:bg-purple-700 disabled:opacity-50 disabled:cursor-not-allowed flex items-center justify-center gap-2 transition-colors"
      >
        {loading ? (
          <>
            <Loader2 size={14} className="animate-spin" />
            Confirming...
          </>
        ) : (
          <>
            <ArrowDownToLine size={14} />
            Deposit {symbol}
          </>
        )}
      </button>

      {status && (
        <div
          className={`rounded-lg px-3 py-2 text-[11px] animate-slide-up ${
            status.type === "success"
              ? "bg-up/10 text-up"
              : "bg-down/10 text-down"
          }`}
        >
          <div className="flex items-start gap-1.5">
            {status.type === "error" && (
              <AlertCircle size={12} className="mt-0.5 shrink-0" />
            )}
            <div>
              <p>{status.msg}</p>
              {status.sig && (
                <a
                  href={`https://explorer.solana.com/tx/${status.sig}?cluster=${process.env.NEXT_PUBLIC_SOLANA_NETWORK || "devnet"}`}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="underline opacity-75 hover:opacity-100"
                >
                  View on Explorer
                </a>
              )}
            </div>
          </div>
        </div>
      )}
    </form>
  );
};

export default SolanaDeposit;
