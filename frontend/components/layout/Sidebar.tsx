import React from "react";
import Link from "next/link";
import { useRouter } from "next/router";
import {
  LayoutDashboard,
  BarChart3,
  Wallet,
  Trophy,
  History,
  Layers,
  Shield,
  ChevronLeft,
} from "lucide-react";
import { useAuth } from "@/contexts/AuthProvider";

interface SidebarProps {
  collapsed: boolean;
  onToggle: () => void;
}

const NAV = [
  { href: "/", label: "Dashboard", icon: LayoutDashboard },
  { href: "/markets", label: "Markets", icon: BarChart3 },
  { href: "/twap", label: "TWAP", icon: Layers },
  { href: "/history", label: "History", icon: History },
  { href: "/account", label: "Account", icon: Wallet },
  { href: "/leaderboard", label: "Leaderboard", icon: Trophy },
];

const ADMIN_NAV = [
  { href: "/admin", label: "Admin", icon: Shield },
];

const Sidebar: React.FC<SidebarProps> = ({ collapsed, onToggle }) => {
  const router = useRouter();
  const { hasRole } = useAuth();
  const showAdmin = hasRole("admin");

  return (
    <aside
      className={`fixed left-0 top-0 z-30 flex h-screen flex-col border-r bg-surface-1 transition-all duration-300 ${
        collapsed ? "w-16" : "w-56"
      }`}
    >
      {/* Logo */}
      <div className="flex h-14 items-center justify-between px-4 border-b">
        {!collapsed && (
          <span className="text-lg font-bold bg-gradient-to-r from-brand-400 to-brand-600 bg-clip-text text-transparent">
            IntentX
          </span>
        )}
        <button
          onClick={onToggle}
          className="btn-ghost !p-1.5 rounded-lg"
          aria-label="Toggle sidebar"
        >
          <ChevronLeft
            size={18}
            className={`transition-transform ${collapsed ? "rotate-180" : ""}`}
          />
        </button>
      </div>

      {/* Nav links */}
      <nav className="flex-1 py-3 space-y-1 px-2">
        {NAV.map(({ href, label, icon: Icon }) => {
          const active =
            href === "/"
              ? router.pathname === "/"
              : router.pathname.startsWith(href);
          return (
            <Link
              key={href}
              href={href}
              className={`flex items-center gap-3 rounded-lg px-3 py-2.5 text-sm font-medium transition-all ${
                active
                  ? "bg-brand-600/10 text-brand-500"
                  : "text-[var(--text-secondary)] hover:bg-surface-2 hover:text-[var(--text-primary)]"
              }`}
              title={collapsed ? label : undefined}
            >
              <Icon size={18} className="shrink-0" />
              {!collapsed && <span>{label}</span>}
            </Link>
          );
        })}

        {/* Admin section */}
        {showAdmin && (
          <>
            <div className="my-2 border-t" />
            {ADMIN_NAV.map(({ href, label, icon: Icon }) => {
              const active = router.pathname.startsWith(href);
              return (
                <Link
                  key={href}
                  href={href}
                  className={`flex items-center gap-3 rounded-lg px-3 py-2.5 text-sm font-medium transition-all ${
                    active
                      ? "bg-brand-600/10 text-brand-500"
                      : "text-[var(--text-secondary)] hover:bg-surface-2 hover:text-[var(--text-primary)]"
                  }`}
                  title={collapsed ? label : undefined}
                >
                  <Icon size={18} className="shrink-0" />
                  {!collapsed && <span>{label}</span>}
                </Link>
              );
            })}
          </>
        )}
      </nav>

      {/* Connection indicator */}
      <div className="px-4 py-3 border-t">
        <div className="flex items-center gap-2">
          <span className="h-2 w-2 rounded-full bg-up animate-pulse" />
          {!collapsed && (
            <span className="text-xs text-[var(--text-muted)]">Connected</span>
          )}
        </div>
      </div>
    </aside>
  );
};

export default Sidebar;
