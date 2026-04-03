import React, { useState } from "react";
import {
  Search,
  ArrowDownToLine,
  ArrowUpFromLine,
} from "lucide-react";
import {
  getAccounts,
  getBalances,
  deposit as apiDeposit,
  withdraw as apiWithdraw,
} from "@/lib/api";
import BalanceCards from "@/components/BalanceCards";

interface Account {
  id: string;
  user_id: string;
  account_type: string;
  created_at: string;
}

interface Balance {
  id: string;
  account_id: string;
  asset: string;
  available_balance: number;
  locked_balance: number;
}

export default function AccountPage() {
  const [userId, setUserId] = useState("");
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [selectedAccount, setSelectedAccount] = useState("");
  const [balances, setBalances] = useState<Balance[]>([]);

  const [asset, setAsset] = useState("USDC");
  const [amount, setAmount] = useState("");
  const [status, setStatus] = useState<{
    type: "success" | "error";
    msg: string;
  } | null>(null);

  const loadAccounts = async () => {
    if (!userId.trim()) return;
    try {
      const data = await getAccounts(userId);
      setAccounts(data || []);
      if (data?.length > 0) {
        setSelectedAccount(data[0].id);
        loadBalances(data[0].id);
      }
    } catch {
      setAccounts([]);
    }
  };

  const loadBalances = async (accountId: string) => {
    try {
      const data = await getBalances(accountId);
      setBalances(data || []);
    } catch {
      setBalances([]);
    }
  };

  const handleAction = async (action: "deposit" | "withdraw") => {
    if (!selectedAccount || !amount) return;
    setStatus(null);
    const fn = action === "deposit" ? apiDeposit : apiWithdraw;
    try {
      await fn({
        account_id: selectedAccount,
        asset,
        amount: Number(amount),
      });
      setStatus({
        type: "success",
        msg: `${action === "deposit" ? "Deposit" : "Withdrawal"} successful`,
      });
      loadBalances(selectedAccount);
      setAmount("");
    } catch (err: any) {
      setStatus({
        type: "error",
        msg: err?.response?.data || `${action} failed`,
      });
    }
  };

  return (
    <div className="space-y-6 max-w-5xl mx-auto animate-fade-in">
      <h1 className="text-2xl font-bold">Account</h1>

      {/* User lookup */}
      <div className="card space-y-3">
        <h3 className="text-sm font-semibold">Find Account</h3>
        <div className="flex gap-2">
          <div className="relative flex-1">
            <Search
              size={16}
              className="absolute left-3 top-1/2 -translate-y-1/2 text-[var(--text-muted)]"
            />
            <input
              className="input !pl-9"
              placeholder="Enter User ID..."
              value={userId}
              onChange={(e) => setUserId(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && loadAccounts()}
            />
          </div>
          <button onClick={loadAccounts} className="btn-primary">
            Search
          </button>
        </div>

        {accounts.length > 0 && (
          <div className="flex gap-2 flex-wrap">
            {accounts.map((a) => (
              <button
                key={a.id}
                onClick={() => {
                  setSelectedAccount(a.id);
                  loadBalances(a.id);
                }}
                className={`rounded-lg px-4 py-2 text-sm font-medium transition-all ${
                  selectedAccount === a.id
                    ? "bg-brand-600 text-white shadow-md shadow-brand-600/20"
                    : "bg-surface-2 text-[var(--text-secondary)] hover:bg-surface-3"
                }`}
              >
                <span className="font-mono text-xs">
                  {a.id.slice(0, 8)}...
                </span>
                <span className="ml-2 text-[var(--text-muted)]">
                  ({a.account_type})
                </span>
              </button>
            ))}
          </div>
        )}
      </div>

      {/* Balances */}
      {selectedAccount && (
        <>
          <div className="space-y-3">
            <h2 className="text-lg font-semibold">Balances</h2>
            <BalanceCards balances={balances} />
          </div>

          {/* Deposit / Withdraw */}
          <div className="card space-y-4">
            <h3 className="text-sm font-semibold">Deposit / Withdraw</h3>
            <div className="flex gap-3 flex-wrap items-end">
              <div>
                <label className="text-xs text-[var(--text-muted)] block mb-1">
                  Asset
                </label>
                <select
                  className="input w-28"
                  value={asset}
                  onChange={(e) => setAsset(e.target.value)}
                >
                  <option>USDC</option>
                  <option>ETH</option>
                  <option>BTC</option>
                  <option>SOL</option>
                </select>
              </div>
              <div>
                <label className="text-xs text-[var(--text-muted)] block mb-1">
                  Amount
                </label>
                <input
                  type="number"
                  className="input w-36 font-mono"
                  placeholder="0"
                  value={amount}
                  onChange={(e) => setAmount(e.target.value)}
                  min="1"
                />
              </div>
              <button
                onClick={() => handleAction("deposit")}
                className="btn-success flex items-center gap-1.5"
              >
                <ArrowDownToLine size={14} />
                Deposit
              </button>
              <button
                onClick={() => handleAction("withdraw")}
                className="btn-danger flex items-center gap-1.5"
              >
                <ArrowUpFromLine size={14} />
                Withdraw
              </button>
            </div>

            {status && (
              <div
                className={`rounded-lg px-3 py-2 text-sm animate-slide-up ${
                  status.type === "success"
                    ? "bg-up/10 text-up"
                    : "bg-down/10 text-down"
                }`}
              >
                {status.msg}
              </div>
            )}
          </div>
        </>
      )}
    </div>
  );
}
