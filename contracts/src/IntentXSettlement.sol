// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {Pausable} from "@openzeppelin/contracts/utils/Pausable.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";

/// @title IntentXSettlement
/// @notice Multi-token settlement vault with authority-gated trade execution.
/// @dev Mirrors the Solana Anchor settlement program:
///      - Users deposit ERC-20 tokens, tracked via internal balances.
///      - Authority settles trades by debiting buyer, crediting seller minus fee.
///      - Users withdraw from their internal balance back to their wallet.
contract IntentXSettlement is Pausable, ReentrancyGuard {
    using SafeERC20 for IERC20;

    // ── Constants ────────────────────────────────────────

    /// Maximum fee: 50% (5000 basis points). Sanity cap.
    uint16 public constant MAX_FEE_BPS = 5000;

    // ── State ────────────────────────────────────────────

    /// Backend signer authorized to execute settlements.
    address public authority;

    /// Fee in basis points (100 = 1%).
    uint16 public feeBps;

    /// Account that receives settlement fees.
    address public feeRecipient;

    /// Lifetime counters.
    uint256 public totalSettlements;
    uint256 public totalVolume;

    /// Internal balance: user => token => amount.
    mapping(address => mapping(address => uint256)) public balances;

    // ── Events ───────────────────────────────────────────

    event Deposit(address indexed user, address indexed token, uint256 amount, uint256 newBalance);
    event Withdraw(address indexed user, address indexed token, uint256 amount, uint256 remaining);
    event Settlement(
        bytes16 indexed fillId,
        address indexed buyer,
        address indexed seller,
        address token,
        uint256 amount,
        uint256 fee,
        uint256 sellerReceives
    );
    event AuthorityUpdated(address indexed oldAuthority, address indexed newAuthority);
    event FeeUpdated(uint16 oldFee, uint16 newFee);
    event FeeRecipientUpdated(address indexed oldRecipient, address indexed newRecipient);

    // ── Errors ───────────────────────────────────────────

    error Unauthorized();
    error ZeroAmount();
    error ZeroAddress();
    error InsufficientBalance(uint256 available, uint256 required);
    error FeeTooHigh(uint16 requested, uint16 max);

    // ── Modifiers ────────────────────────────────────────

    modifier onlyAuthority() {
        if (msg.sender != authority) revert Unauthorized();
        _;
    }

    // ── Constructor ──────────────────────────────────────

    constructor(address _authority, address _feeRecipient, uint16 _feeBps) {
        if (_authority == address(0)) revert ZeroAddress();
        if (_feeRecipient == address(0)) revert ZeroAddress();
        if (_feeBps > MAX_FEE_BPS) revert FeeTooHigh(_feeBps, MAX_FEE_BPS);

        authority = _authority;
        feeRecipient = _feeRecipient;
        feeBps = _feeBps;
    }

    // ── Deposit ──────────────────────────────────────────

    /// @notice Deposit ERC-20 tokens into the settlement vault.
    /// @param token The ERC-20 token contract address.
    /// @param amount Amount to deposit (in token's smallest unit).
    function deposit(address token, uint256 amount) external nonReentrant whenNotPaused {
        if (amount == 0) revert ZeroAmount();
        if (token == address(0)) revert ZeroAddress();

        IERC20(token).safeTransferFrom(msg.sender, address(this), amount);

        balances[msg.sender][token] += amount;

        emit Deposit(msg.sender, token, amount, balances[msg.sender][token]);
    }

    // ── Withdraw ─────────────────────────────────────────

    /// @notice Withdraw tokens from internal balance back to wallet.
    /// @param token The ERC-20 token contract address.
    /// @param amount Amount to withdraw.
    function withdraw(address token, uint256 amount) external nonReentrant whenNotPaused {
        if (amount == 0) revert ZeroAmount();

        uint256 bal = balances[msg.sender][token];
        if (bal < amount) revert InsufficientBalance(bal, amount);

        balances[msg.sender][token] = bal - amount;

        IERC20(token).safeTransfer(msg.sender, amount);

        emit Withdraw(msg.sender, token, amount, balances[msg.sender][token]);
    }

    // ── Settlement ───────────────────────────────────────

    /// @notice Execute a settlement between buyer and seller.
    /// @dev Only callable by the authorized backend signer.
    /// @param buyer Address whose balance is debited.
    /// @param seller Address whose balance is credited (minus fee).
    /// @param token The ERC-20 token being settled.
    /// @param amount Total amount to transfer from buyer.
    /// @param fillId 16-byte fill identifier for off-chain correlation.
    function settle(
        address buyer,
        address seller,
        address token,
        uint256 amount,
        bytes16 fillId
    ) external onlyAuthority nonReentrant whenNotPaused {
        if (amount == 0) revert ZeroAmount();

        // Calculate fee
        uint256 fee = (amount * feeBps) / 10_000;
        uint256 sellerReceives = amount - fee;

        // Debit buyer
        uint256 buyerBal = balances[buyer][token];
        if (buyerBal < amount) revert InsufficientBalance(buyerBal, amount);
        balances[buyer][token] = buyerBal - amount;

        // Credit seller
        balances[seller][token] += sellerReceives;

        // Credit fee recipient
        if (fee > 0) {
            balances[feeRecipient][token] += fee;
        }

        // Update stats
        totalSettlements++;
        totalVolume += amount;

        emit Settlement(fillId, buyer, seller, token, amount, fee, sellerReceives);
    }

    // ── Admin ────────────────────────────────────────────

    /// @notice Transfer authority to a new address.
    function updateAuthority(address newAuthority) external onlyAuthority {
        if (newAuthority == address(0)) revert ZeroAddress();
        emit AuthorityUpdated(authority, newAuthority);
        authority = newAuthority;
    }

    /// @notice Update the settlement fee.
    function updateFee(uint16 newFeeBps) external onlyAuthority {
        if (newFeeBps > MAX_FEE_BPS) revert FeeTooHigh(newFeeBps, MAX_FEE_BPS);
        emit FeeUpdated(feeBps, newFeeBps);
        feeBps = newFeeBps;
    }

    /// @notice Update the fee recipient address.
    function updateFeeRecipient(address newRecipient) external onlyAuthority {
        if (newRecipient == address(0)) revert ZeroAddress();
        emit FeeRecipientUpdated(feeRecipient, newRecipient);
        feeRecipient = newRecipient;
    }

    /// @notice Pause all deposits, withdrawals, and settlements.
    function pause() external onlyAuthority {
        _pause();
    }

    /// @notice Unpause the contract.
    function unpause() external onlyAuthority {
        _unpause();
    }
}
