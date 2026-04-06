#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────
# anchor_deploy.sh — Build, deploy, and verify IntentX Solana programs
#
# Deploys:
#   1. intentx-settlement  (deposit / settle / withdraw vault)
#   2. intentx-htlc        (hash time-locked contracts)
#
# After deployment it:
#   - Writes program IDs into Anchor.toml and declare_id!() macros
#   - Updates frontend/lib/solana.ts with the settlement program ID
#   - Creates frontend/lib/solana-config.ts with all deployed addresses
#   - Initializes the settlement program (config PDA, vault token account)
#   - Runs anchor test against the live deployment
#
# Usage:
#   # Devnet (default)
#   ./scripts/anchor_deploy.sh
#
#   # Mainnet
#   CLUSTER=mainnet-beta ./scripts/anchor_deploy.sh
#
#   # Custom RPC
#   CLUSTER=devnet RPC_URL=https://my-rpc.example.com ./scripts/anchor_deploy.sh
#
# Environment variables:
#   CLUSTER            devnet | mainnet-beta | localnet (default: devnet)
#   RPC_URL            custom RPC endpoint (overrides cluster default)
#   DEPLOYER_KEYPAIR   path to deployer keypair (default: ~/.config/solana/id.json)
#   AUTHORITY_KEYPAIR  path to settlement authority keypair (default: DEPLOYER_KEYPAIR)
#   FEE_BPS            settlement fee basis points (default: 10 = 0.1%)
#   SKIP_INIT          set to 1 to skip settlement initialization
#   SKIP_TEST          set to 1 to skip anchor tests
#   SKIP_FRONTEND      set to 1 to skip frontend config update
# ─────────────────────────────────────────────────────────────

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
PROGRAMS_DIR="$ROOT_DIR/programs"
FRONTEND_DIR="$ROOT_DIR/frontend"
KEYS_DIR="$ROOT_DIR/.keys"

CLUSTER="${CLUSTER:-devnet}"
DEPLOYER_KEYPAIR="${DEPLOYER_KEYPAIR:-$HOME/.config/solana/id.json}"
AUTHORITY_KEYPAIR="${AUTHORITY_KEYPAIR:-$DEPLOYER_KEYPAIR}"
FEE_BPS="${FEE_BPS:-10}"
SKIP_INIT="${SKIP_INIT:-0}"
SKIP_TEST="${SKIP_TEST:-0}"
SKIP_FRONTEND="${SKIP_FRONTEND:-0}"

# Resolve RPC URL
if [ -n "${RPC_URL:-}" ]; then
    SOLANA_RPC="$RPC_URL"
elif [ "$CLUSTER" = "mainnet-beta" ]; then
    SOLANA_RPC="https://api.mainnet-beta.solana.com"
elif [ "$CLUSTER" = "localnet" ]; then
    SOLANA_RPC="http://127.0.0.1:8899"
else
    SOLANA_RPC="https://api.devnet.solana.com"
fi

# ── Helpers ──────────────────────────────────────────────

log()  { echo "==> $*"; }
warn() { echo "WARN: $*" >&2; }
die()  { echo "ERROR: $*" >&2; exit 1; }

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || die "$1 is required but not found in PATH"
}

# ── Preflight checks ────────────────────────────────────

require_cmd solana
require_cmd anchor
require_cmd jq

[ -f "$DEPLOYER_KEYPAIR" ] || die "Deployer keypair not found: $DEPLOYER_KEYPAIR"

DEPLOYER_PUBKEY="$(solana-keygen pubkey "$DEPLOYER_KEYPAIR")"
AUTHORITY_PUBKEY="$(solana-keygen pubkey "$AUTHORITY_KEYPAIR")"

log "Cluster:    $CLUSTER"
log "RPC:        $SOLANA_RPC"
log "Deployer:   $DEPLOYER_PUBKEY"
log "Authority:  $AUTHORITY_PUBKEY"
log "Fee BPS:    $FEE_BPS"

# Set solana config for this session
solana config set --url "$SOLANA_RPC" --keypair "$DEPLOYER_KEYPAIR" >/dev/null

# Check deployer balance
BALANCE="$(solana balance --lamports "$DEPLOYER_PUBKEY" 2>/dev/null | awk '{print $1}')"
if [ "$CLUSTER" != "localnet" ] && [ "${BALANCE:-0}" -lt 1000000000 ]; then
    warn "Deployer balance is low ($(solana balance "$DEPLOYER_PUBKEY")). Deployment may fail."
    if [ "$CLUSTER" = "devnet" ]; then
        log "Requesting devnet airdrop..."
        solana airdrop 2 "$DEPLOYER_PUBKEY" --url "$SOLANA_RPC" || warn "Airdrop failed — continue anyway"
        sleep 2
    fi
fi

# ── Step 1: Generate program keypairs ────────────────────

mkdir -p "$KEYS_DIR"

generate_program_keypair() {
    local name="$1"
    local keyfile="$KEYS_DIR/${name}-keypair.json"

    if [ -f "$keyfile" ]; then
        log "Using existing keypair for $name"
    else
        log "Generating new keypair for $name"
        solana-keygen new --no-bip39-passphrase --outfile "$keyfile" --force --silent
    fi

    solana-keygen pubkey "$keyfile"
}

SETTLEMENT_PROGRAM_ID="$(generate_program_keypair intentx-settlement)"
HTLC_PROGRAM_ID="$(generate_program_keypair intentx-htlc)"

log "Settlement program ID: $SETTLEMENT_PROGRAM_ID"
log "HTLC program ID:       $HTLC_PROGRAM_ID"

# ── Step 2: Update declare_id!() in program source ───────

update_declare_id() {
    local file="$1"
    local program_id="$2"

    if [ -f "$file" ]; then
        sed -i.bak "s/declare_id!(\"[^\"]*\")/declare_id!(\"$program_id\")/" "$file"
        rm -f "${file}.bak"
        log "Updated declare_id in $file"
    fi
}

update_declare_id "$PROGRAMS_DIR/intentx-settlement/src/lib.rs" "$SETTLEMENT_PROGRAM_ID"
update_declare_id "$PROGRAMS_DIR/intentx-htlc/src/lib.rs" "$HTLC_PROGRAM_ID"

# ── Step 3: Update Anchor.toml ───────────────────────────

ANCHOR_TOML="$PROGRAMS_DIR/intentx-settlement/Anchor.toml"

cat > "$ANCHOR_TOML" <<TOML
[toolchain]

[features]
seeds = false
skip-lint = false

[programs.localnet]
intentx_settlement = "$SETTLEMENT_PROGRAM_ID"
intentx_htlc = "$HTLC_PROGRAM_ID"

[programs.devnet]
intentx_settlement = "$SETTLEMENT_PROGRAM_ID"
intentx_htlc = "$HTLC_PROGRAM_ID"

[programs.mainnet]
intentx_settlement = "$SETTLEMENT_PROGRAM_ID"
intentx_htlc = "$HTLC_PROGRAM_ID"

[registry]
url = "https://api.apr.dev"

[provider]
cluster = "$CLUSTER"
wallet = "$DEPLOYER_KEYPAIR"

[scripts]
test = "yarn run ts-mocha -p ./tsconfig.json -t 1000000 tests/**/*.ts"
TOML

log "Updated Anchor.toml"

# ── Step 4: Build programs ───────────────────────────────

log "Building programs..."

cd "$PROGRAMS_DIR/intentx-settlement"
anchor build 2>&1 | tail -5

cd "$PROGRAMS_DIR/intentx-htlc"
anchor build 2>&1 | tail -5

# ── Step 5: Deploy programs ──────────────────────────────

deploy_program() {
    local name="$1"
    local program_dir="$2"
    local keypair="$KEYS_DIR/${name}-keypair.json"
    local so_file

    # Find the .so file — anchor build puts it in target/deploy/
    so_file="$(find "$program_dir/target/deploy" -name "*.so" 2>/dev/null | head -1)"
    if [ -z "$so_file" ]; then
        # Try workspace-level target
        so_file="$(find "$ROOT_DIR/target/deploy" -name "${name//-/_}.so" 2>/dev/null | head -1)"
    fi

    [ -n "$so_file" ] || die "Could not find .so for $name"

    log "Deploying $name from $so_file ..."
    solana program deploy \
        --url "$SOLANA_RPC" \
        --keypair "$DEPLOYER_KEYPAIR" \
        --program-id "$keypair" \
        "$so_file"
}

deploy_program "intentx-settlement" "$PROGRAMS_DIR/intentx-settlement"
deploy_program "intentx-htlc" "$PROGRAMS_DIR/intentx-htlc"

log "Both programs deployed successfully"

# ── Step 6: Write deployment manifest ────────────────────

MANIFEST="$ROOT_DIR/.keys/deployment-${CLUSTER}.json"

cat > "$MANIFEST" <<JSON
{
  "cluster": "$CLUSTER",
  "rpc_url": "$SOLANA_RPC",
  "deployer": "$DEPLOYER_PUBKEY",
  "authority": "$AUTHORITY_PUBKEY",
  "fee_bps": $FEE_BPS,
  "programs": {
    "intentx_settlement": "$SETTLEMENT_PROGRAM_ID",
    "intentx_htlc": "$HTLC_PROGRAM_ID"
  },
  "deployed_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
JSON

log "Deployment manifest written to $MANIFEST"

# ── Step 7: Update frontend constants ────────────────────

if [ "$SKIP_FRONTEND" = "1" ]; then
    log "Skipping frontend config update (SKIP_FRONTEND=1)"
else
    # Update the PROGRAM_ID default in solana.ts
    SOLANA_TS="$FRONTEND_DIR/lib/solana.ts"
    if [ -f "$SOLANA_TS" ]; then
        sed -i.bak \
            "s|\"11111111111111111111111111111111\"|\"$SETTLEMENT_PROGRAM_ID\"|g" \
            "$SOLANA_TS"
        rm -f "${SOLANA_TS}.bak"
        log "Updated settlement program ID in frontend/lib/solana.ts"
    fi

    # Generate a full config file with all addresses
    CONFIG_TS="$FRONTEND_DIR/lib/solana-config.ts"
    cat > "$CONFIG_TS" <<TS
// Auto-generated by scripts/anchor_deploy.sh — do not edit manually.
// Cluster: $CLUSTER
// Deployed: $(date -u +%Y-%m-%dT%H:%M:%SZ)

export const SOLANA_CONFIG = {
  cluster: "$CLUSTER" as const,
  rpcUrl: process.env.NEXT_PUBLIC_SOLANA_RPC_URL || "$SOLANA_RPC",

  // Program IDs
  settlementProgramId: "$SETTLEMENT_PROGRAM_ID",
  htlcProgramId: "$HTLC_PROGRAM_ID",

  // Authority (backend signer for settlements)
  authority: "$AUTHORITY_PUBKEY",

  // Fee configuration
  feeBps: $FEE_BPS,
} as const;

export type SolanaCluster = typeof SOLANA_CONFIG.cluster;
TS

    log "Generated frontend/lib/solana-config.ts"
fi

# ── Step 8: Initialize settlement program ────────────────

if [ "$SKIP_INIT" = "1" ]; then
    log "Skipping settlement initialization (SKIP_INIT=1)"
else
    log "Initializing settlement program (fee_bps=$FEE_BPS)..."

    # The initialize instruction is called via a small Anchor test/script.
    # We write a temporary Node.js script that uses @coral-xyz/anchor.
    INIT_SCRIPT="$(mktemp /tmp/init_settlement_XXXXXX.ts)"
    trap "rm -f '$INIT_SCRIPT'" EXIT

    cat > "$INIT_SCRIPT" <<'INITTS'
import * as anchor from "@coral-xyz/anchor";
import { PublicKey, Keypair } from "@solana/web3.js";
import fs from "fs";

async function main() {
    const cluster = process.env.CLUSTER || "devnet";
    const rpcUrl = process.env.SOLANA_RPC!;
    const deployerPath = process.env.DEPLOYER_KEYPAIR!;
    const authorityPath = process.env.AUTHORITY_KEYPAIR || deployerPath;
    const feeBps = parseInt(process.env.FEE_BPS || "10");
    const programId = new PublicKey(process.env.SETTLEMENT_PROGRAM_ID!);

    const connection = new anchor.web3.Connection(rpcUrl, "confirmed");
    const deployerSecret = JSON.parse(fs.readFileSync(deployerPath, "utf-8"));
    const deployer = Keypair.fromSecretKey(Uint8Array.from(deployerSecret));
    const authoritySecret = JSON.parse(fs.readFileSync(authorityPath, "utf-8"));
    const authority = Keypair.fromSecretKey(Uint8Array.from(authoritySecret));

    // Check if config PDA already exists (already initialized)
    const [configPda] = PublicKey.findProgramAddressSync(
        [Buffer.from("config")],
        programId
    );

    const existing = await connection.getAccountInfo(configPda);
    if (existing) {
        console.log("Settlement program already initialized. Config PDA:", configPda.toBase58());
        return;
    }

    console.log("Config PDA:", configPda.toBase58());
    console.log("Initializing with authority:", authority.publicKey.toBase58(), "fee_bps:", feeBps);

    // Build initialize instruction manually (Anchor discriminator + args)
    const crypto = require("crypto");
    const disc = crypto.createHash("sha256").update("global:initialize").digest().subarray(0, 8);
    const data = Buffer.alloc(10); // 8 disc + 2 fee_bps
    disc.copy(data, 0);
    data.writeUInt16LE(feeBps, 8);

    const ix = new anchor.web3.TransactionInstruction({
        programId,
        keys: [
            { pubkey: configPda, isSigner: false, isWritable: true },
            { pubkey: authority.publicKey, isSigner: true, isWritable: true },
            { pubkey: authority.publicKey, isSigner: false, isWritable: false }, // fee_recipient
            { pubkey: anchor.web3.SystemProgram.programId, isSigner: false, isWritable: false },
        ],
        data,
    });

    const tx = new anchor.web3.Transaction().add(ix);
    tx.feePayer = deployer.publicKey;
    const { blockhash } = await connection.getLatestBlockhash();
    tx.recentBlockhash = blockhash;

    // Sign with all required signers
    const signers = [deployer];
    if (authority.publicKey.toBase58() !== deployer.publicKey.toBase58()) {
        signers.push(authority);
    }
    tx.sign(...signers);

    const sig = await connection.sendRawTransaction(tx.serialize());
    await connection.confirmTransaction(sig, "confirmed");
    console.log("Initialized! tx:", sig);
}

main().catch((e) => { console.error(e); process.exit(1); });
INITTS

    # Run the init script with required env vars
    CLUSTER="$CLUSTER" \
    SOLANA_RPC="$SOLANA_RPC" \
    DEPLOYER_KEYPAIR="$DEPLOYER_KEYPAIR" \
    AUTHORITY_KEYPAIR="$AUTHORITY_KEYPAIR" \
    FEE_BPS="$FEE_BPS" \
    SETTLEMENT_PROGRAM_ID="$SETTLEMENT_PROGRAM_ID" \
        npx tsx "$INIT_SCRIPT" || warn "Settlement initialization failed — you may need to initialize manually"
fi

# ── Step 9: Run verification tests ───────────────────────

if [ "$SKIP_TEST" = "1" ]; then
    log "Skipping tests (SKIP_TEST=1)"
else
    log "Running Anchor program tests..."
    cd "$PROGRAMS_DIR/intentx-settlement"
    anchor test --skip-deploy --provider.cluster "$CLUSTER" 2>&1 | tail -20 \
        || warn "Settlement tests had failures — check output above"

    cd "$PROGRAMS_DIR/intentx-htlc"
    anchor test --skip-deploy --provider.cluster "$CLUSTER" 2>&1 | tail -20 \
        || warn "HTLC tests had failures — check output above"
fi

# ── Done ─────────────────────────────────────────────────

echo ""
log "╔══════════════════════════════════════════════════════════╗"
log "║            IntentX Solana Deployment Complete            ║"
log "╠══════════════════════════════════════════════════════════╣"
log "║ Cluster:    $CLUSTER"
log "║ Settlement: $SETTLEMENT_PROGRAM_ID"
log "║ HTLC:       $HTLC_PROGRAM_ID"
log "║ Authority:  $AUTHORITY_PUBKEY"
log "║ Fee BPS:    $FEE_BPS"
log "║ Manifest:   $MANIFEST"
log "╚══════════════════════════════════════════════════════════╝"
echo ""
log "Frontend env vars for .env.local:"
echo "  NEXT_PUBLIC_SETTLEMENT_PROGRAM_ID=$SETTLEMENT_PROGRAM_ID"
echo "  NEXT_PUBLIC_HTLC_PROGRAM_ID=$HTLC_PROGRAM_ID"
echo "  NEXT_PUBLIC_SOLANA_NETWORK=$CLUSTER"
echo "  NEXT_PUBLIC_SOLANA_RPC_URL=$SOLANA_RPC"
