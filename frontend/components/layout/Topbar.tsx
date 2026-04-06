import React from "react";
import { Sun, Moon, Bell, Search, LogOut, User } from "lucide-react";
import { useTheme } from "@/hooks/useTheme";
import { useWebSocket } from "@/contexts/WebSocketProvider";
import { useAuth } from "@/contexts/AuthProvider";
import WalletButton from "@/components/solana-wallet/WalletButton";

const Topbar: React.FC = () => {
  const { dark, toggle } = useTheme();
  const { connected } = useWebSocket();
  const { user, logout } = useAuth();

  return (
    <header className="sticky top-0 z-20 flex h-14 items-center justify-between border-b bg-surface-1/80 backdrop-blur-md px-6">
      {/* Search */}
      <div className="relative w-72">
        <Search
          size={16}
          className="absolute left-3 top-1/2 -translate-y-1/2 text-[var(--text-muted)]"
        />
        <input
          className="input !pl-9 !py-2 !text-sm"
          placeholder="Search markets, intents..."
        />
      </div>

      {/* Right side */}
      <div className="flex items-center gap-3">
        {/* WS status */}
        <span
          className={`badge ${connected ? "badge-success" : "badge-danger"}`}
        >
          <span
            className={`mr-1.5 h-1.5 w-1.5 rounded-full ${
              connected ? "bg-up" : "bg-down"
            }`}
          />
          {connected ? "Live" : "Offline"}
        </span>

        {/* Solana wallet */}
        <WalletButton />

        {/* Notifications */}
        <button className="btn-ghost !p-2 relative" aria-label="Notifications">
          <Bell size={18} />
          <span className="absolute top-1 right-1 h-2 w-2 rounded-full bg-brand-500" />
        </button>

        {/* Theme toggle */}
        <button
          onClick={toggle}
          className="btn-ghost !p-2"
          aria-label="Toggle theme"
        >
          {dark ? <Sun size={18} /> : <Moon size={18} />}
        </button>

        {/* User info + logout */}
        {user && (
          <div className="flex items-center gap-2 border-l pl-3">
            <div className="h-7 w-7 rounded-full bg-brand-600/20 flex items-center justify-center">
              <User size={14} className="text-brand-400" />
            </div>
            <div className="hidden sm:block">
              <p className="text-xs font-medium leading-none truncate max-w-[120px]">
                {user.email}
              </p>
              <p className="text-[10px] text-[var(--text-muted)]">
                {user.roles.join(", ")}
              </p>
            </div>
            <button
              onClick={logout}
              className="btn-ghost !p-1.5"
              aria-label="Logout"
              title="Sign out"
            >
              <LogOut size={16} />
            </button>
          </div>
        )}
      </div>
    </header>
  );
};

export default Topbar;
