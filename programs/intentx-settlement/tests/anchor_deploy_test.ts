/**
 * Post-deployment verification tests for IntentX Solana programs.
 *
 * Run after anchor_deploy.sh to verify on-chain state:
 *   npx tsx programs/intentx-settlement/tests/anchor_deploy_test.ts
 *
 * Or via anchor:
 *   cd programs/intentx-settlement && anchor test --skip-deploy
 *
 * Env vars:
 *   SOLANA_RPC                  RPC endpoint (default: devnet)
 *   DEPLOYER_KEYPAIR            path to deployer keypair
 *   SETTLEMENT_PROGRAM_ID       deployed settlement program ID
 *   HTLC_PROGRAM_ID             deployed HTLC program ID
 */

import {
  Connection,
  Keypair,
  PublicKey,
  Transaction,
  TransactionInstruction,
  SystemProgram,
  sendAndConfirmTransaction,
} from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  createMint,
  createAccount,
  mintTo,
  getAccount,
  getAssociatedTokenAddress,
  createAssociatedTokenAccountInstruction,
} from "@solana/spl-token";
import * as crypto from "crypto";
import * as fs from "fs";
import * as assert from "assert";

// ── Config ──────────────────────────────────────────────

const RPC_URL = process.env.SOLANA_RPC || "https://api.devnet.solana.com";
const DEPLOYER_PATH =
  process.env.DEPLOYER_KEYPAIR || `${process.env.HOME}/.config/solana/id.json`;

const SETTLEMENT_ID = new PublicKey(
  process.env.SETTLEMENT_PROGRAM_ID || "11111111111111111111111111111111"
);
const HTLC_ID = new PublicKey(
  process.env.HTLC_PROGRAM_ID || "HtLc1111111111111111111111111111111111111111"
);

// ── Helpers ─────────────────────────────────────────────

function loadKeypair(path: string): Keypair {
  const raw = JSON.parse(fs.readFileSync(path, "utf-8"));
  return Keypair.fromSecretKey(Uint8Array.from(raw));
}

function anchorDisc(name: string): Buffer {
  return crypto
    .createHash("sha256")
    .update(`global:${name}`)
    .digest()
    .subarray(0, 8);
}

function deriveConfigPDA(): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("config")],
    SETTLEMENT_ID
  );
}

function deriveUserPDA(
  owner: PublicKey,
  mint: PublicKey
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("user"), owner.toBuffer(), mint.toBuffer()],
    SETTLEMENT_ID
  );
}

function deriveVaultAuthority(config: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("vault"), config.toBuffer()],
    SETTLEMENT_ID
  );
}

function deriveHtlcPDA(hashlock: Buffer): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("htlc"), hashlock],
    HTLC_ID
  );
}

function deriveEscrowPDA(hashlock: Buffer): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("escrow"), hashlock],
    HTLC_ID
  );
}

// ── Test runner ─────────────────────────────────────────

async function main() {
  const connection = new Connection(RPC_URL, "confirmed");
  const deployer = loadKeypair(DEPLOYER_PATH);

  console.log("=== IntentX Deployment Verification ===");
  console.log("RPC:        ", RPC_URL);
  console.log("Deployer:   ", deployer.publicKey.toBase58());
  console.log("Settlement: ", SETTLEMENT_ID.toBase58());
  console.log("HTLC:       ", HTLC_ID.toBase58());
  console.log("");

  let passed = 0;
  let failed = 0;

  async function test(name: string, fn: () => Promise<void>) {
    try {
      await fn();
      console.log(`  ✓ ${name}`);
      passed++;
    } catch (e: any) {
      console.log(`  ✗ ${name}`);
      console.log(`    ${e.message || e}`);
      failed++;
    }
  }

  // ── Settlement program tests ──────────────────────────

  console.log("--- Settlement Program ---");

  await test("program is deployed", async () => {
    const info = await connection.getAccountInfo(SETTLEMENT_ID);
    assert.ok(info, "Settlement program account not found");
    assert.ok(info.executable, "Account is not executable");
  });

  const [configPda] = deriveConfigPDA();

  await test("config PDA is initialized", async () => {
    const info = await connection.getAccountInfo(configPda);
    assert.ok(info, "Config PDA not found — run initialize first");
    // 8 disc + 32 authority + 2 fee_bps + 32 fee_recipient + 8 total_settlements + 8 total_volume + 1 bump = 91
    assert.ok(info.data.length >= 91, `Config data too small: ${info.data.length}`);
  });

  await test("config authority is set", async () => {
    const info = await connection.getAccountInfo(configPda);
    assert.ok(info);
    // Authority is at offset 8 (after discriminator), 32 bytes
    const authority = new PublicKey(info.data.subarray(8, 40));
    assert.ok(
      !authority.equals(PublicKey.default),
      "Authority is zero pubkey"
    );
    console.log(`    authority: ${authority.toBase58()}`);
  });

  await test("config fee_bps is reasonable", async () => {
    const info = await connection.getAccountInfo(configPda);
    assert.ok(info);
    // fee_bps is at offset 40 (8 disc + 32 authority), 2 bytes LE
    const feeBps = info.data.readUInt16LE(40);
    assert.ok(feeBps <= 5000, `Fee too high: ${feeBps} bps`);
    console.log(`    fee_bps: ${feeBps}`);
  });

  // ── Deposit + withdraw flow (if on devnet/localnet) ───

  const isLive = RPC_URL.includes("mainnet");

  if (!isLive) {
    console.log("");
    console.log("--- Settlement Deposit/Withdraw Flow ---");

    let mint: PublicKey;
    let vaultTokenAccount: PublicKey;

    await test("create test SPL mint", async () => {
      mint = await createMint(
        connection,
        deployer,
        deployer.publicKey,
        null,
        6
      );
      assert.ok(mint, "Failed to create mint");
      console.log(`    mint: ${mint.toBase58()}`);
    });

    await test("create vault token account", async () => {
      const [vaultAuth] = deriveVaultAuthority(configPda);
      vaultTokenAccount = await createAccount(
        connection,
        deployer,
        mint!,
        vaultAuth
      );
      assert.ok(vaultTokenAccount, "Failed to create vault token account");
      console.log(`    vault: ${vaultTokenAccount.toBase58()}`);
    });

    let userAta: PublicKey;

    await test("create user ATA and mint tokens", async () => {
      userAta = await getAssociatedTokenAddress(mint!, deployer.publicKey);

      const tx = new Transaction().add(
        createAssociatedTokenAccountInstruction(
          deployer.publicKey,
          userAta,
          deployer.publicKey,
          mint!
        )
      );
      await sendAndConfirmTransaction(connection, tx, [deployer]);

      await mintTo(
        connection,
        deployer,
        mint!,
        userAta,
        deployer.publicKey,
        1_000_000 // 1 token with 6 decimals
      );

      const account = await getAccount(connection, userAta);
      assert.equal(account.amount.toString(), "1000000");
    });

    await test("deposit 500_000 into vault", async () => {
      const [userPda] = deriveUserPDA(deployer.publicKey, mint!);

      const disc = anchorDisc("deposit");
      const data = Buffer.alloc(16);
      disc.copy(data, 0);
      data.writeBigUInt64LE(500_000n, 8);

      const ix = new TransactionInstruction({
        programId: SETTLEMENT_ID,
        keys: [
          { pubkey: configPda, isSigner: false, isWritable: false },
          { pubkey: userPda, isSigner: false, isWritable: true },
          { pubkey: deployer.publicKey, isSigner: true, isWritable: true },
          { pubkey: mint!, isSigner: false, isWritable: false },
          { pubkey: userAta!, isSigner: false, isWritable: true },
          { pubkey: vaultTokenAccount!, isSigner: false, isWritable: true },
          { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
          { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
        ],
        data,
      });

      const tx = new Transaction().add(ix);
      const sig = await sendAndConfirmTransaction(connection, tx, [deployer]);
      console.log(`    tx: ${sig}`);

      // Verify vault received tokens
      const vaultAcct = await getAccount(connection, vaultTokenAccount!);
      assert.equal(vaultAcct.amount.toString(), "500000");
    });

    await test("withdraw 200_000 from vault", async () => {
      const [userPda] = deriveUserPDA(deployer.publicKey, mint!);
      const [vaultAuth] = deriveVaultAuthority(configPda);

      const disc = anchorDisc("withdraw");
      const data = Buffer.alloc(16);
      disc.copy(data, 0);
      data.writeBigUInt64LE(200_000n, 8);

      const ix = new TransactionInstruction({
        programId: SETTLEMENT_ID,
        keys: [
          { pubkey: configPda, isSigner: false, isWritable: false },
          { pubkey: userPda, isSigner: false, isWritable: true },
          { pubkey: deployer.publicKey, isSigner: true, isWritable: true },
          { pubkey: mint!, isSigner: false, isWritable: false },
          { pubkey: userAta!, isSigner: false, isWritable: true },
          { pubkey: vaultTokenAccount!, isSigner: false, isWritable: true },
          { pubkey: vaultAuth, isSigner: false, isWritable: false },
          { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
        ],
        data,
      });

      const tx = new Transaction().add(ix);
      const sig = await sendAndConfirmTransaction(connection, tx, [deployer]);
      console.log(`    tx: ${sig}`);

      // Vault should have 300_000 remaining
      const vaultAcct = await getAccount(connection, vaultTokenAccount!);
      assert.equal(vaultAcct.amount.toString(), "300000");

      // User should have 700_000 (1M - 500K deposited + 200K withdrawn)
      const userAcct = await getAccount(connection, userAta!);
      assert.equal(userAcct.amount.toString(), "700000");
    });
  }

  // ── HTLC program tests ────────────────────────────────

  console.log("");
  console.log("--- HTLC Program ---");

  await test("program is deployed", async () => {
    const info = await connection.getAccountInfo(HTLC_ID);
    assert.ok(info, "HTLC program account not found");
    assert.ok(info.executable, "Account is not executable");
  });

  await test("PDA derivation is deterministic", async () => {
    const secret = Buffer.alloc(32, 0xab);
    const hashlock = crypto.createHash("sha256").update(secret).digest();

    const [htlc1] = deriveHtlcPDA(hashlock);
    const [htlc2] = deriveHtlcPDA(hashlock);
    assert.ok(htlc1.equals(htlc2), "HTLC PDAs not deterministic");

    const [escrow1] = deriveEscrowPDA(hashlock);
    const [escrow2] = deriveEscrowPDA(hashlock);
    assert.ok(escrow1.equals(escrow2), "Escrow PDAs not deterministic");

    assert.ok(!htlc1.equals(escrow1), "HTLC and Escrow PDAs should differ");
  });

  if (!isLive) {
    console.log("");
    console.log("--- HTLC Lock/Claim Flow ---");

    let htlcMint: PublicKey;
    const secret = crypto.randomBytes(32);
    const hashlock = crypto.createHash("sha256").update(secret).digest();
    const receiver = Keypair.generate();

    await test("create HTLC test mint and fund sender", async () => {
      htlcMint = await createMint(
        connection,
        deployer,
        deployer.publicKey,
        null,
        6
      );

      const senderAta = await getAssociatedTokenAddress(htlcMint, deployer.publicKey);
      const tx = new Transaction().add(
        createAssociatedTokenAccountInstruction(
          deployer.publicKey,
          senderAta,
          deployer.publicKey,
          htlcMint
        )
      );
      await sendAndConfirmTransaction(connection, tx, [deployer]);

      await mintTo(
        connection,
        deployer,
        htlcMint,
        senderAta,
        deployer.publicKey,
        5_000_000
      );
    });

    await test("lock_funds into HTLC escrow", async () => {
      const [htlcPda] = deriveHtlcPDA(hashlock);
      const [escrowPda] = deriveEscrowPDA(hashlock);
      const senderAta = await getAssociatedTokenAddress(htlcMint!, deployer.publicKey);

      const timelock = Math.floor(Date.now() / 1000) + 3600; // 1 hour from now

      const disc = anchorDisc("lock_funds");
      const data = Buffer.alloc(8 + 32 + 8 + 8); // disc + hashlock + timelock + amount
      disc.copy(data, 0);
      hashlock.copy(data, 8);
      data.writeBigInt64LE(BigInt(timelock), 40);
      data.writeBigUInt64LE(1_000_000n, 48);

      const ix = new TransactionInstruction({
        programId: HTLC_ID,
        keys: [
          { pubkey: htlcPda, isSigner: false, isWritable: true },
          { pubkey: escrowPda, isSigner: false, isWritable: true },
          { pubkey: deployer.publicKey, isSigner: true, isWritable: true },
          { pubkey: receiver.publicKey, isSigner: false, isWritable: false },
          { pubkey: htlcMint!, isSigner: false, isWritable: false },
          { pubkey: senderAta, isSigner: false, isWritable: true },
          { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
          { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
        ],
        data,
      });

      const tx = new Transaction().add(ix);
      const sig = await sendAndConfirmTransaction(connection, tx, [deployer]);
      console.log(`    tx: ${sig}`);

      // Verify HTLC state
      const htlcInfo = await connection.getAccountInfo(htlcPda);
      assert.ok(htlcInfo, "HTLC account not created");
      assert.ok(htlcInfo.data.length >= 156, "HTLC data too small");
    });

    await test("claim HTLC with secret", async () => {
      const [htlcPda] = deriveHtlcPDA(hashlock);
      const [escrowPda] = deriveEscrowPDA(hashlock);

      // Fund receiver for ATA creation
      const tx0 = new Transaction().add(
        SystemProgram.transfer({
          fromPubkey: deployer.publicKey,
          toPubkey: receiver.publicKey,
          lamports: 10_000_000,
        })
      );
      await sendAndConfirmTransaction(connection, tx0, [deployer]);

      const receiverAta = await getAssociatedTokenAddress(htlcMint!, receiver.publicKey);
      const ataIx = createAssociatedTokenAccountInstruction(
        deployer.publicKey,
        receiverAta,
        receiver.publicKey,
        htlcMint!
      );

      const disc = anchorDisc("claim");
      const data = Buffer.alloc(8 + 32); // disc + secret
      disc.copy(data, 0);
      secret.copy(data, 8);

      const claimIx = new TransactionInstruction({
        programId: HTLC_ID,
        keys: [
          { pubkey: htlcPda, isSigner: false, isWritable: true },
          { pubkey: escrowPda, isSigner: false, isWritable: true },
          { pubkey: receiverAta, isSigner: false, isWritable: true },
          { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
        ],
        data,
      });

      const tx = new Transaction().add(ataIx, claimIx);
      const sig = await sendAndConfirmTransaction(connection, tx, [deployer]);
      console.log(`    tx: ${sig}`);

      // Verify receiver got the tokens
      const receiverAcct = await getAccount(connection, receiverAta);
      assert.equal(receiverAcct.amount.toString(), "1000000");
    });
  }

  // ── Summary ───────────────────────────────────────────

  console.log("");
  console.log(`=== Results: ${passed} passed, ${failed} failed ===`);

  if (failed > 0) {
    process.exit(1);
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
