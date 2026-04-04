import React, { useState } from "react";
import Link from "next/link";
import { useAuth } from "@/contexts/AuthProvider";
import { LogIn } from "lucide-react";

export default function LoginPage() {
  const { login } = useAuth();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    setLoading(true);
    try {
      await login(email, password);
    } catch (err: any) {
      setError(
        err?.response?.data?.toString() || err?.message || "Login failed"
      );
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="min-h-[80vh] flex items-center justify-center">
      <form
        onSubmit={handleSubmit}
        className="card w-full max-w-sm space-y-4 p-6"
      >
        <div className="flex items-center gap-2 mb-2">
          <div className="h-10 w-10 rounded-xl bg-brand-600/10 flex items-center justify-center">
            <LogIn size={20} className="text-brand-400" />
          </div>
          <h1 className="text-xl font-bold">Sign In</h1>
        </div>

        <div className="space-y-1">
          <label className="text-xs font-medium text-[var(--text-muted)]">
            Email
          </label>
          <input
            type="email"
            className="input"
            placeholder="you@example.com"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            required
            autoFocus
          />
        </div>

        <div className="space-y-1">
          <label className="text-xs font-medium text-[var(--text-muted)]">
            Password
          </label>
          <input
            type="password"
            className="input"
            placeholder="Enter password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
            minLength={6}
          />
        </div>

        {error && (
          <div className="rounded-lg bg-down/10 px-3 py-2 text-sm text-down animate-slide-up">
            {error}
          </div>
        )}

        <button type="submit" disabled={loading} className="btn-primary w-full">
          {loading ? "Signing in..." : "Sign In"}
        </button>

        <p className="text-center text-sm text-[var(--text-muted)]">
          Don't have an account?{" "}
          <Link href="/register" className="text-brand-400 hover:underline">
            Register
          </Link>
        </p>
      </form>
    </div>
  );
}
