import React, { useState } from "react";
import Link from "next/link";
import { useAuth } from "@/contexts/AuthProvider";
import { UserPlus } from "lucide-react";

export default function RegisterPage() {
  const { register } = useAuth();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");

    if (password !== confirm) {
      setError("Passwords do not match");
      return;
    }
    if (password.length < 6) {
      setError("Password must be at least 6 characters");
      return;
    }

    setLoading(true);
    try {
      await register(email, password);
    } catch (err: any) {
      setError(
        err?.response?.data?.toString() || err?.message || "Registration failed"
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
          <div className="h-10 w-10 rounded-xl bg-up/10 flex items-center justify-center">
            <UserPlus size={20} className="text-up" />
          </div>
          <h1 className="text-xl font-bold">Create Account</h1>
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
            placeholder="Min 6 characters"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
            minLength={6}
          />
        </div>

        <div className="space-y-1">
          <label className="text-xs font-medium text-[var(--text-muted)]">
            Confirm Password
          </label>
          <input
            type="password"
            className="input"
            placeholder="Repeat password"
            value={confirm}
            onChange={(e) => setConfirm(e.target.value)}
            required
          />
        </div>

        {error && (
          <div className="rounded-lg bg-down/10 px-3 py-2 text-sm text-down animate-slide-up">
            {error}
          </div>
        )}

        <button type="submit" disabled={loading} className="btn-success w-full">
          {loading ? "Creating account..." : "Create Account"}
        </button>

        <p className="text-center text-sm text-[var(--text-muted)]">
          Already have an account?{" "}
          <Link href="/login" className="text-brand-400 hover:underline">
            Sign in
          </Link>
        </p>
      </form>
    </div>
  );
}
