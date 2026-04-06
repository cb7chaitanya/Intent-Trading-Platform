// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test, console} from "forge-std/Test.sol";
import {IntentXSettlement} from "../src/IntentXSettlement.sol";
import {MockERC20} from "./mocks/MockERC20.sol";

contract IntentXSettlementTest is Test {
    IntentXSettlement public vault;
    MockERC20 public usdc;
    MockERC20 public weth;

    address authority = makeAddr("authority");
    address feeRecipient = makeAddr("feeRecipient");
    address alice = makeAddr("alice");
    address bob = makeAddr("bob");
    address charlie = makeAddr("charlie");

    uint16 constant FEE_BPS = 100; // 1%

    function setUp() public {
        vault = new IntentXSettlement(authority, feeRecipient, FEE_BPS);

        usdc = new MockERC20("USD Coin", "USDC", 6);
        weth = new MockERC20("Wrapped Ether", "WETH", 18);

        // Mint tokens to users
        usdc.mint(alice, 1_000_000e6);
        usdc.mint(bob, 500_000e6);
        weth.mint(alice, 100e18);

        // Approve vault
        vm.prank(alice);
        usdc.approve(address(vault), type(uint256).max);
        vm.prank(alice);
        weth.approve(address(vault), type(uint256).max);
        vm.prank(bob);
        usdc.approve(address(vault), type(uint256).max);
    }

    // ── Constructor ──────────────────────────────────

    function test_constructor_sets_state() public view {
        assertEq(vault.authority(), authority);
        assertEq(vault.feeRecipient(), feeRecipient);
        assertEq(vault.feeBps(), FEE_BPS);
        assertEq(vault.totalSettlements(), 0);
        assertEq(vault.totalVolume(), 0);
    }

    function test_constructor_reverts_zero_authority() public {
        vm.expectRevert(IntentXSettlement.ZeroAddress.selector);
        new IntentXSettlement(address(0), feeRecipient, FEE_BPS);
    }

    function test_constructor_reverts_zero_fee_recipient() public {
        vm.expectRevert(IntentXSettlement.ZeroAddress.selector);
        new IntentXSettlement(authority, address(0), FEE_BPS);
    }

    function test_constructor_reverts_fee_too_high() public {
        vm.expectRevert(abi.encodeWithSelector(
            IntentXSettlement.FeeTooHigh.selector, 5001, 5000
        ));
        new IntentXSettlement(authority, feeRecipient, 5001);
    }

    // ── Deposit ──────────────────────────────────────

    function test_deposit() public {
        vm.prank(alice);
        vault.deposit(address(usdc), 10_000e6);

        assertEq(vault.balances(alice, address(usdc)), 10_000e6);
        assertEq(usdc.balanceOf(address(vault)), 10_000e6);
    }

    function test_deposit_emits_event() public {
        vm.expectEmit(true, true, false, true);
        emit IntentXSettlement.Deposit(alice, address(usdc), 10_000e6, 10_000e6);

        vm.prank(alice);
        vault.deposit(address(usdc), 10_000e6);
    }

    function test_deposit_accumulates() public {
        vm.startPrank(alice);
        vault.deposit(address(usdc), 5_000e6);
        vault.deposit(address(usdc), 3_000e6);
        vm.stopPrank();

        assertEq(vault.balances(alice, address(usdc)), 8_000e6);
    }

    function test_deposit_multiple_tokens() public {
        vm.startPrank(alice);
        vault.deposit(address(usdc), 10_000e6);
        vault.deposit(address(weth), 5e18);
        vm.stopPrank();

        assertEq(vault.balances(alice, address(usdc)), 10_000e6);
        assertEq(vault.balances(alice, address(weth)), 5e18);
    }

    function test_deposit_reverts_zero_amount() public {
        vm.prank(alice);
        vm.expectRevert(IntentXSettlement.ZeroAmount.selector);
        vault.deposit(address(usdc), 0);
    }

    function test_deposit_reverts_zero_token() public {
        vm.prank(alice);
        vm.expectRevert(IntentXSettlement.ZeroAddress.selector);
        vault.deposit(address(0), 1000);
    }

    // ── Withdraw ─────────────────────────────────────

    function test_withdraw() public {
        vm.prank(alice);
        vault.deposit(address(usdc), 10_000e6);

        uint256 balBefore = usdc.balanceOf(alice);

        vm.prank(alice);
        vault.withdraw(address(usdc), 4_000e6);

        assertEq(vault.balances(alice, address(usdc)), 6_000e6);
        assertEq(usdc.balanceOf(alice), balBefore + 4_000e6);
    }

    function test_withdraw_full_balance() public {
        vm.prank(alice);
        vault.deposit(address(usdc), 10_000e6);

        vm.prank(alice);
        vault.withdraw(address(usdc), 10_000e6);

        assertEq(vault.balances(alice, address(usdc)), 0);
    }

    function test_withdraw_emits_event() public {
        vm.prank(alice);
        vault.deposit(address(usdc), 10_000e6);

        vm.expectEmit(true, true, false, true);
        emit IntentXSettlement.Withdraw(alice, address(usdc), 4_000e6, 6_000e6);

        vm.prank(alice);
        vault.withdraw(address(usdc), 4_000e6);
    }

    function test_withdraw_reverts_insufficient() public {
        vm.prank(alice);
        vault.deposit(address(usdc), 10_000e6);

        vm.prank(alice);
        vm.expectRevert(abi.encodeWithSelector(
            IntentXSettlement.InsufficientBalance.selector, 10_000e6, 20_000e6
        ));
        vault.withdraw(address(usdc), 20_000e6);
    }

    function test_withdraw_reverts_zero() public {
        vm.prank(alice);
        vm.expectRevert(IntentXSettlement.ZeroAmount.selector);
        vault.withdraw(address(usdc), 0);
    }

    // ── Settlement ───────────────────────────────────

    function test_settle() public {
        // Alice deposits as buyer
        vm.prank(alice);
        vault.deposit(address(usdc), 10_000e6);

        bytes16 fillId = bytes16(uint128(42));

        vm.prank(authority);
        vault.settle(alice, bob, address(usdc), 10_000e6, fillId);

        // 1% fee: 100 USDC
        assertEq(vault.balances(alice, address(usdc)), 0);
        assertEq(vault.balances(bob, address(usdc)), 9_900e6);
        assertEq(vault.balances(feeRecipient, address(usdc)), 100e6);

        assertEq(vault.totalSettlements(), 1);
        assertEq(vault.totalVolume(), 10_000e6);
    }

    function test_settle_emits_event() public {
        vm.prank(alice);
        vault.deposit(address(usdc), 5_000e6);

        bytes16 fillId = bytes16(uint128(99));

        vm.expectEmit(true, true, true, true);
        emit IntentXSettlement.Settlement(
            fillId, alice, bob, address(usdc), 5_000e6, 50e6, 4_950e6
        );

        vm.prank(authority);
        vault.settle(alice, bob, address(usdc), 5_000e6, fillId);
    }

    function test_settle_zero_fee() public {
        // Deploy vault with 0% fee
        IntentXSettlement noFeeVault = new IntentXSettlement(authority, feeRecipient, 0);

        vm.prank(alice);
        usdc.approve(address(noFeeVault), type(uint256).max);
        vm.prank(alice);
        noFeeVault.deposit(address(usdc), 1_000e6);

        vm.prank(authority);
        noFeeVault.settle(alice, bob, address(usdc), 1_000e6, bytes16(0));

        assertEq(noFeeVault.balances(bob, address(usdc)), 1_000e6);
        assertEq(noFeeVault.balances(feeRecipient, address(usdc)), 0);
    }

    function test_settle_reverts_non_authority() public {
        vm.prank(alice);
        vault.deposit(address(usdc), 1_000e6);

        vm.prank(bob);
        vm.expectRevert(IntentXSettlement.Unauthorized.selector);
        vault.settle(alice, bob, address(usdc), 1_000e6, bytes16(0));
    }

    function test_settle_reverts_insufficient_buyer_balance() public {
        vm.prank(alice);
        vault.deposit(address(usdc), 500e6);

        vm.prank(authority);
        vm.expectRevert(abi.encodeWithSelector(
            IntentXSettlement.InsufficientBalance.selector, 500e6, 1_000e6
        ));
        vault.settle(alice, bob, address(usdc), 1_000e6, bytes16(0));
    }

    function test_settle_reverts_zero_amount() public {
        vm.prank(authority);
        vm.expectRevert(IntentXSettlement.ZeroAmount.selector);
        vault.settle(alice, bob, address(usdc), 0, bytes16(0));
    }

    function test_settle_multiple() public {
        vm.prank(alice);
        vault.deposit(address(usdc), 20_000e6);

        vm.startPrank(authority);
        vault.settle(alice, bob, address(usdc), 10_000e6, bytes16(uint128(1)));
        vault.settle(alice, charlie, address(usdc), 5_000e6, bytes16(uint128(2)));
        vm.stopPrank();

        assertEq(vault.balances(alice, address(usdc)), 5_000e6);
        assertEq(vault.balances(bob, address(usdc)), 9_900e6);
        assertEq(vault.balances(charlie, address(usdc)), 4_950e6);
        assertEq(vault.balances(feeRecipient, address(usdc)), 150e6);
        assertEq(vault.totalSettlements(), 2);
        assertEq(vault.totalVolume(), 15_000e6);
    }

    // ── Fee math ─────────────────────────────────────

    function test_fee_calculation_1_percent() public {
        vm.prank(alice);
        vault.deposit(address(usdc), 10_000e6);

        vm.prank(authority);
        vault.settle(alice, bob, address(usdc), 10_000e6, bytes16(0));

        // 10000 * 100 / 10000 = 100
        assertEq(vault.balances(feeRecipient, address(usdc)), 100e6);
        assertEq(vault.balances(bob, address(usdc)), 9_900e6);
    }

    function test_fee_calculation_small_amount() public {
        vm.prank(alice);
        vault.deposit(address(usdc), 99);

        vm.prank(authority);
        vault.settle(alice, bob, address(usdc), 99, bytes16(0));

        // 99 * 100 / 10000 = 0 (rounds down)
        assertEq(vault.balances(feeRecipient, address(usdc)), 0);
        assertEq(vault.balances(bob, address(usdc)), 99);
    }

    function test_fee_calculation_max_bps() public {
        IntentXSettlement maxFeeVault = new IntentXSettlement(authority, feeRecipient, 5000);

        vm.prank(alice);
        usdc.approve(address(maxFeeVault), type(uint256).max);
        vm.prank(alice);
        maxFeeVault.deposit(address(usdc), 10_000e6);

        vm.prank(authority);
        maxFeeVault.settle(alice, bob, address(usdc), 10_000e6, bytes16(0));

        // 50% fee
        assertEq(maxFeeVault.balances(feeRecipient, address(usdc)), 5_000e6);
        assertEq(maxFeeVault.balances(bob, address(usdc)), 5_000e6);
    }

    // ── Admin ────────────────────────────────────────

    function test_update_authority() public {
        vm.prank(authority);
        vault.updateAuthority(charlie);
        assertEq(vault.authority(), charlie);
    }

    function test_update_authority_reverts_non_authority() public {
        vm.prank(bob);
        vm.expectRevert(IntentXSettlement.Unauthorized.selector);
        vault.updateAuthority(bob);
    }

    function test_update_authority_reverts_zero() public {
        vm.prank(authority);
        vm.expectRevert(IntentXSettlement.ZeroAddress.selector);
        vault.updateAuthority(address(0));
    }

    function test_update_fee() public {
        vm.prank(authority);
        vault.updateFee(250); // 2.5%
        assertEq(vault.feeBps(), 250);
    }

    function test_update_fee_reverts_too_high() public {
        vm.prank(authority);
        vm.expectRevert(abi.encodeWithSelector(
            IntentXSettlement.FeeTooHigh.selector, 5001, 5000
        ));
        vault.updateFee(5001);
    }

    function test_update_fee_recipient() public {
        vm.prank(authority);
        vault.updateFeeRecipient(charlie);
        assertEq(vault.feeRecipient(), charlie);
    }

    // ── Pausable ─────────────────────────────────────

    function test_pause_blocks_deposit() public {
        vm.prank(authority);
        vault.pause();

        vm.prank(alice);
        vm.expectRevert();
        vault.deposit(address(usdc), 1000);
    }

    function test_pause_blocks_withdraw() public {
        vm.prank(alice);
        vault.deposit(address(usdc), 1000);

        vm.prank(authority);
        vault.pause();

        vm.prank(alice);
        vm.expectRevert();
        vault.withdraw(address(usdc), 1000);
    }

    function test_pause_blocks_settle() public {
        vm.prank(alice);
        vault.deposit(address(usdc), 1000);

        vm.prank(authority);
        vault.pause();

        vm.prank(authority);
        vm.expectRevert();
        vault.settle(alice, bob, address(usdc), 1000, bytes16(0));
    }

    function test_unpause_resumes_operations() public {
        vm.prank(authority);
        vault.pause();
        vm.prank(authority);
        vault.unpause();

        vm.prank(alice);
        vault.deposit(address(usdc), 1000);
        assertEq(vault.balances(alice, address(usdc)), 1000);
    }

    function test_pause_only_authority() public {
        vm.prank(bob);
        vm.expectRevert(IntentXSettlement.Unauthorized.selector);
        vault.pause();
    }

    // ── Full flow ────────────────────────────────────

    function test_full_settlement_flow() public {
        // 1. Alice deposits 10,000 USDC
        vm.prank(alice);
        vault.deposit(address(usdc), 10_000e6);

        // 2. Settle: Alice buys from Bob, 10,000 USDC at 1% fee
        vm.prank(authority);
        vault.settle(alice, bob, address(usdc), 10_000e6, bytes16(uint128(42)));

        // 3. Bob withdraws his 9,900 USDC
        uint256 bobBalBefore = usdc.balanceOf(bob);
        vm.prank(bob);
        vault.withdraw(address(usdc), 9_900e6);
        assertEq(usdc.balanceOf(bob), bobBalBefore + 9_900e6);

        // 4. Fee recipient withdraws 100 USDC fee
        vm.prank(feeRecipient);
        vault.withdraw(address(usdc), 100e6);

        // 5. Vault is empty
        assertEq(vault.balances(alice, address(usdc)), 0);
        assertEq(vault.balances(bob, address(usdc)), 0);
        assertEq(vault.balances(feeRecipient, address(usdc)), 0);
        assertEq(usdc.balanceOf(address(vault)), 0);
    }
}
