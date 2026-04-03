import React from "react";
import SolversLeaderboard from "@/components/SolversLeaderboard";

export default function LeaderboardPage() {
  return (
    <div className="space-y-6 max-w-3xl mx-auto animate-fade-in">
      <h1 className="text-2xl font-bold">Solver Leaderboard</h1>
      <SolversLeaderboard limit={50} />
    </div>
  );
}
