import {
  Connection,
  PublicKey,
  Transaction,
  TransactionInstruction,
  SystemProgram,
} from "@solana/web3.js";
import {
  getAssociatedTokenAddress,
  createAssociatedTokenAccountInstruction,
  TOKEN_PROGRAM_ID,
  getAccount,
} from "@solana/spl-token";

// ── Constants ────────────────────────────────────────────

const PROGRAM_ID = new PublicKey(
  process.env.NEXT_PUBLIC_SETTLEMENT_PROGRAM_ID ||
    "11111111111111111111111111111111"
);

const USER_SEED = Buffer.from("user");
const CONFIG_SEED = Buffer.from("config");
const VAULT_SEED = Buffer.from("vault");

// ── PDA helpers ──────────────────────────────────────────

export function deriveConfigPDA(): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([CONFIG_SEED], PROGRAM_ID);
}

export function deriveUserAccountPDA(
  owner: PublicKey,
  mint: PublicKey
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [USER_SEED, owner.toBuffer(), mint.toBuffer()],
    PROGRAM_ID
  );
}

export function deriveVaultAuthorityPDA(
  config: PublicKey
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [VAULT_SEED, config.toBuffer()],
    PROGRAM_ID
  );
}

// ── Discriminator ──────────���─────────────────────────────

function anchorDiscSync(name: string): Buffer {
  const full = `global:${name}`;
  // Use Node.js crypto (available in Next.js)
  const crypto = require("crypto");
  const hash = crypto.createHash("sha256").update(full).digest();
  return Buffer.from(hash.subarray(0, 8));
}

// ── Build deposit instruction ────────────────────────────

export async function buildDepositInstruction(
  user: PublicKey,
  mint: PublicKey,
  userTokenAccount: PublicKey,
  vaultTokenAccount: PublicKey,
  amount: bigint
): Promise<TransactionInstruction> {
  const [config] = deriveConfigPDA();
  const [userAccount] = deriveUserAccountPDA(user, mint);

  const disc = anchorDiscSync("deposit");
  const data = Buffer.alloc(16);
  disc.copy(data, 0);
  data.writeBigUInt64LE(amount, 8);

  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: config, isSigner: false, isWritable: false },
      { pubkey: userAccount, isSigner: false, isWritable: true },
      { pubkey: user, isSigner: true, isWritable: true },
      { pubkey: mint, isSigner: false, isWritable: false },
      { pubkey: userTokenAccount, isSigner: false, isWritable: true },
      { pubkey: vaultTokenAccount, isSigner: false, isWritable: true },
      { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
    ],
    data,
  });
}

// ── Build withdraw instruction ──────��────────────────────

export async function buildWithdrawInstruction(
  user: PublicKey,
  mint: PublicKey,
  userTokenAccount: PublicKey,
  vaultTokenAccount: PublicKey,
  amount: bigint
): Promise<TransactionInstruction> {
  const [config] = deriveConfigPDA();
  const [userAccount] = deriveUserAccountPDA(user, mint);
  const [vaultAuthority] = deriveVaultAuthorityPDA(config);

  const disc = anchorDiscSync("withdraw");
  const data = Buffer.alloc(16);
  disc.copy(data, 0);
  data.writeBigUInt64LE(amount, 8);

  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys: [
      { pubkey: config, isSigner: false, isWritable: false },
      { pubkey: userAccount, isSigner: false, isWritable: true },
      { pubkey: user, isSigner: true, isWritable: true },
      { pubkey: mint, isSigner: false, isWritable: false },
      { pubkey: userTokenAccount, isSigner: false, isWritable: true },
      { pubkey: vaultTokenAccount, isSigner: false, isWritable: true },
      { pubkey: vaultAuthority, isSigner: false, isWritable: false },
      { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
    ],
    data,
  });
}

// ── Send deposit transaction ────��────────────────────────

export interface DepositParams {
  connection: Connection;
  wallet: PublicKey;
  mint: PublicKey;
  vaultTokenAccount: PublicKey;
  amount: bigint;
  signTransaction: (tx: Transaction) => Promise<Transaction>;
}

export async function sendDeposit(params: DepositParams): Promise<string> {
  const {
    connection,
    wallet,
    mint,
    vaultTokenAccount,
    amount,
    signTransaction,
  } = params;

  const tx = new Transaction();

  // Ensure the user's ATA exists
  const userAta = await getAssociatedTokenAddress(mint, wallet);
  try {
    await getAccount(connection, userAta);
  } catch {
    tx.add(
      createAssociatedTokenAccountInstruction(wallet, userAta, wallet, mint)
    );
  }

  // Build the deposit instruction
  const depositIx = await buildDepositInstruction(
    wallet,
    mint,
    userAta,
    vaultTokenAccount,
    amount
  );
  tx.add(depositIx);

  // Set recent blockhash and fee payer
  const { blockhash, lastValidBlockHeight } =
    await connection.getLatestBlockhash("confirmed");
  tx.recentBlockhash = blockhash;
  tx.lastValidBlockHeight = lastValidBlockHeight;
  tx.feePayer = wallet;

  // Sign and send
  const signed = await signTransaction(tx);
  const signature = await connection.sendRawTransaction(signed.serialize(), {
    skipPreflight: false,
    preflightCommitment: "confirmed",
  });

  // Wait for confirmation
  await connection.confirmTransaction(
    { signature, blockhash, lastValidBlockHeight },
    "confirmed"
  );

  return signature;
}
