// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {IntentXSettlement} from "../src/IntentXSettlement.sol";
import {DeploySettlement, DeployTestnet, MockERC20Deployer} from "../script/Deploy.s.sol";

// ── DeploySettlement tests ──────────────────────────────

contract DeploySettlementTest is Test {
    DeploySettlement deployer;

    function setUp() public {
        deployer = new DeploySettlement();
    }

    function test_deploy_default_params() public {
        address authority = makeAddr("authority");
        address feeRecipient = makeAddr("feeRecipient");

        IntentXSettlement s = deployer.deploy(authority, feeRecipient, 10);

        assertEq(s.authority(), authority);
        assertEq(s.feeRecipient(), feeRecipient);
        assertEq(s.feeBps(), 10);
        assertEq(s.totalSettlements(), 0);
        assertEq(s.totalVolume(), 0);
    }

    function test_deploy_zero_fee() public {
        address a = makeAddr("a");
        IntentXSettlement s = deployer.deploy(a, a, 0);
        assertEq(s.feeBps(), 0);
    }

    function test_deploy_max_fee() public {
        address a = makeAddr("a");
        IntentXSettlement s = deployer.deploy(a, a, 5000);
        assertEq(s.feeBps(), 5000);
    }

    function test_deploy_reverts_fee_too_high() public {
        address a = makeAddr("a");
        vm.expectRevert(abi.encodeWithSelector(
            IntentXSettlement.FeeTooHigh.selector, 5001, 5000
        ));
        deployer.deploy(a, a, 5001);
    }

    function test_deploy_reverts_zero_authority() public {
        vm.expectRevert(IntentXSettlement.ZeroAddress.selector);
        deployer.deploy(address(0), makeAddr("f"), 10);
    }

    function test_deploy_reverts_zero_fee_recipient() public {
        vm.expectRevert(IntentXSettlement.ZeroAddress.selector);
        deployer.deploy(makeAddr("a"), address(0), 10);
    }

    function test_deploy_different_params_different_addresses() public {
        address a = makeAddr("a");
        IntentXSettlement s1 = deployer.deploy(a, a, 10);
        IntentXSettlement s2 = deployer.deploy(a, a, 20);

        // Each deployment creates a new contract at a different address
        assertTrue(address(s1) != address(s2));
    }

    function test_deployed_contract_is_functional() public {
        address auth = makeAddr("auth");
        address fees = makeAddr("fees");
        IntentXSettlement s = deployer.deploy(auth, fees, 100);

        MockERC20Deployer token = new MockERC20Deployer("Test", "TST", 18);
        address user = makeAddr("user");
        token.mint(user, 1000e18);

        vm.startPrank(user);
        token.approve(address(s), type(uint256).max);
        s.deposit(address(token), 500e18);
        vm.stopPrank();

        assertEq(s.balances(user, address(token)), 500e18);
    }

    function test_deployed_contract_not_paused() public {
        address a = makeAddr("a");
        IntentXSettlement s = deployer.deploy(a, a, 10);

        MockERC20Deployer token = new MockERC20Deployer("T", "T", 18);
        address user = makeAddr("user");
        token.mint(user, 1000);

        vm.startPrank(user);
        token.approve(address(s), type(uint256).max);
        s.deposit(address(token), 100); // should not revert
        vm.stopPrank();
    }

    function test_deploy_settlement_then_settle() public {
        address auth = makeAddr("auth");
        address fees = makeAddr("fees");
        address alice = makeAddr("alice");
        address bob = makeAddr("bob");

        IntentXSettlement s = deployer.deploy(auth, fees, 100); // 1%
        MockERC20Deployer token = new MockERC20Deployer("USDC", "USDC", 6);
        token.mint(alice, 10_000e6);

        vm.startPrank(alice);
        token.approve(address(s), type(uint256).max);
        s.deposit(address(token), 10_000e6);
        vm.stopPrank();

        vm.prank(auth);
        s.settle(alice, bob, address(token), 10_000e6, bytes16(uint128(1)));

        assertEq(s.balances(bob, address(token)), 9_900e6);
        assertEq(s.balances(fees, address(token)), 100e6);
        assertEq(s.totalSettlements(), 1);
        assertEq(s.totalVolume(), 10_000e6);
    }
}

// ── DeployTestnet tests ─────────────────────────────────

contract DeployTestnetTest is Test {
    DeployTestnet testnetDeployer;

    function setUp() public {
        testnetDeployer = new DeployTestnet();
    }

    function test_deploys_all_three_contracts() public {
        address auth = makeAddr("auth");
        address fees = makeAddr("fees");
        address alice = makeAddr("alice");
        address bob = makeAddr("bob");

        (
            IntentXSettlement settlement,
            MockERC20Deployer usdc,
            MockERC20Deployer weth
        ) = testnetDeployer.deployAll(auth, fees, 100, alice, bob, 1_000_000e6, 100e18);

        assertTrue(address(settlement) != address(0));
        assertTrue(address(usdc) != address(0));
        assertTrue(address(weth) != address(0));
    }

    function test_settlement_params() public {
        address auth = makeAddr("auth");
        address fees = makeAddr("fees");
        address alice = makeAddr("alice");

        (IntentXSettlement settlement,,) =
            testnetDeployer.deployAll(auth, fees, 50, alice, alice, 0, 0);

        assertEq(settlement.authority(), auth);
        assertEq(settlement.feeRecipient(), fees);
        assertEq(settlement.feeBps(), 50);
    }

    function test_mock_token_metadata() public {
        address a = makeAddr("a");

        (, MockERC20Deployer usdc, MockERC20Deployer weth) =
            testnetDeployer.deployAll(a, a, 10, a, a, 0, 0);

        assertEq(usdc.name(), "USD Coin");
        assertEq(usdc.symbol(), "USDC");
        assertEq(usdc.decimals(), 6);

        assertEq(weth.name(), "Wrapped Ether");
        assertEq(weth.symbol(), "WETH");
        assertEq(weth.decimals(), 18);
    }

    function test_mints_to_alice_and_bob() public {
        address a = makeAddr("a");
        address alice = makeAddr("alice");
        address bob = makeAddr("bob");

        (, MockERC20Deployer usdc, MockERC20Deployer weth) =
            testnetDeployer.deployAll(a, a, 10, alice, bob, 1_000_000e6, 100e18);

        assertEq(usdc.balanceOf(alice), 1_000_000e6);
        assertEq(usdc.balanceOf(bob), 1_000_000e6);
        assertEq(weth.balanceOf(alice), 100e18);
        assertEq(weth.balanceOf(bob), 100e18);
    }

    function test_custom_mint_amounts() public {
        address a = makeAddr("a");
        address alice = makeAddr("alice");
        address bob = makeAddr("bob");

        (, MockERC20Deployer usdc, MockERC20Deployer weth) =
            testnetDeployer.deployAll(a, a, 10, alice, bob, 500_000e6, 50e18);

        assertEq(usdc.balanceOf(alice), 500_000e6);
        assertEq(usdc.balanceOf(bob), 500_000e6);
        assertEq(weth.balanceOf(alice), 50e18);
        assertEq(weth.balanceOf(bob), 50e18);

        // Total supply = 2 * mint_amount
        assertEq(usdc.totalSupply(), 1_000_000e6);
        assertEq(weth.totalSupply(), 100e18);
    }

    function test_same_user_gets_double() public {
        address a = makeAddr("a");
        // When alice == bob, the same address gets minted twice
        (, MockERC20Deployer usdc,) =
            testnetDeployer.deployAll(a, a, 10, a, a, 1_000e6, 1e18);

        assertEq(usdc.balanceOf(a), 2_000e6); // minted twice
    }

    function test_full_flow_after_testnet_deploy() public {
        address auth = makeAddr("auth");
        address fees = makeAddr("fees");
        address alice = makeAddr("alice");
        address bob = makeAddr("bob");

        (
            IntentXSettlement settlement,
            MockERC20Deployer usdc,
        ) = testnetDeployer.deployAll(auth, fees, 100, alice, bob, 10_000e6, 0);

        // Alice deposits and settles with Bob
        vm.startPrank(alice);
        usdc.approve(address(settlement), type(uint256).max);
        settlement.deposit(address(usdc), 5_000e6);
        vm.stopPrank();

        vm.prank(auth);
        settlement.settle(alice, bob, address(usdc), 5_000e6, bytes16(uint128(42)));

        // 1% fee
        assertEq(settlement.balances(bob, address(usdc)), 4_950e6);
        assertEq(settlement.balances(fees, address(usdc)), 50e6);

        // Bob withdraws
        vm.prank(bob);
        settlement.withdraw(address(usdc), 4_950e6);
        assertEq(usdc.balanceOf(bob), 10_000e6 + 4_950e6); // initial mint + withdrawal
    }
}

// ── MockERC20Deployer unit tests ────────────────────────

contract MockERC20DeployerTest is Test {
    MockERC20Deployer usdc;
    MockERC20Deployer weth;

    function setUp() public {
        usdc = new MockERC20Deployer("USD Coin", "USDC", 6);
        weth = new MockERC20Deployer("Wrapped Ether", "WETH", 18);
    }

    function test_metadata() public view {
        assertEq(usdc.name(), "USD Coin");
        assertEq(usdc.symbol(), "USDC");
        assertEq(usdc.decimals(), 6);
        assertEq(weth.name(), "Wrapped Ether");
        assertEq(weth.symbol(), "WETH");
        assertEq(weth.decimals(), 18);
    }

    function test_mint() public {
        address alice = makeAddr("alice");
        usdc.mint(alice, 1_000_000e6);
        assertEq(usdc.balanceOf(alice), 1_000_000e6);
        assertEq(usdc.totalSupply(), 1_000_000e6);
    }

    function test_mint_multiple_users() public {
        address alice = makeAddr("alice");
        address bob = makeAddr("bob");
        usdc.mint(alice, 1_000_000e6);
        usdc.mint(bob, 500_000e6);
        assertEq(usdc.balanceOf(alice), 1_000_000e6);
        assertEq(usdc.balanceOf(bob), 500_000e6);
        assertEq(usdc.totalSupply(), 1_500_000e6);
    }

    function test_mint_accumulates() public {
        address alice = makeAddr("alice");
        usdc.mint(alice, 100e6);
        usdc.mint(alice, 200e6);
        assertEq(usdc.balanceOf(alice), 300e6);
    }

    function test_transfer() public {
        address alice = makeAddr("alice");
        address bob = makeAddr("bob");
        usdc.mint(alice, 1000);

        vm.prank(alice);
        assertTrue(usdc.transfer(bob, 400));

        assertEq(usdc.balanceOf(alice), 600);
        assertEq(usdc.balanceOf(bob), 400);
    }

    function test_transfer_reverts_insufficient() public {
        address alice = makeAddr("alice");
        address bob = makeAddr("bob");
        usdc.mint(alice, 100);

        vm.prank(alice);
        vm.expectRevert("insufficient balance");
        usdc.transfer(bob, 200);
    }

    function test_approve_and_transfer_from() public {
        address alice = makeAddr("alice");
        address bob = makeAddr("bob");
        address spender = makeAddr("spender");

        usdc.mint(alice, 1000);
        vm.prank(alice);
        usdc.approve(spender, 500);
        assertEq(usdc.allowance(alice, spender), 500);

        vm.prank(spender);
        assertTrue(usdc.transferFrom(alice, bob, 300));

        assertEq(usdc.balanceOf(alice), 700);
        assertEq(usdc.balanceOf(bob), 300);
        assertEq(usdc.allowance(alice, spender), 200);
    }

    function test_transfer_from_reverts_insufficient_allowance() public {
        address alice = makeAddr("alice");
        address bob = makeAddr("bob");
        address spender = makeAddr("spender");

        usdc.mint(alice, 1000);
        vm.prank(alice);
        usdc.approve(spender, 100);

        vm.prank(spender);
        vm.expectRevert("insufficient allowance");
        usdc.transferFrom(alice, bob, 200);
    }

    function test_works_with_settlement_vault() public {
        address authority = makeAddr("authority");
        address feeRecipient = makeAddr("feeRecipient");
        address alice = makeAddr("alice");
        address bob = makeAddr("bob");

        IntentXSettlement vault = new IntentXSettlement(authority, feeRecipient, 100);
        usdc.mint(alice, 10_000e6);

        vm.startPrank(alice);
        usdc.approve(address(vault), type(uint256).max);
        vault.deposit(address(usdc), 5_000e6);
        vm.stopPrank();

        assertEq(vault.balances(alice, address(usdc)), 5_000e6);
        assertEq(usdc.balanceOf(address(vault)), 5_000e6);

        vm.prank(authority);
        vault.settle(alice, bob, address(usdc), 5_000e6, bytes16(uint128(1)));

        assertEq(vault.balances(bob, address(usdc)), 4_950e6);
        assertEq(vault.balances(feeRecipient, address(usdc)), 50e6);

        vm.prank(bob);
        vault.withdraw(address(usdc), 4_950e6);
        assertEq(usdc.balanceOf(bob), 4_950e6);
    }

    function test_multiple_tokens_with_vault() public {
        address authority = makeAddr("authority");
        address feeRecipient = makeAddr("feeRecipient");
        address alice = makeAddr("alice");

        IntentXSettlement vault = new IntentXSettlement(authority, feeRecipient, 0);
        usdc.mint(alice, 10_000e6);
        weth.mint(alice, 10e18);

        vm.startPrank(alice);
        usdc.approve(address(vault), type(uint256).max);
        weth.approve(address(vault), type(uint256).max);
        vault.deposit(address(usdc), 5_000e6);
        vault.deposit(address(weth), 2e18);
        vm.stopPrank();

        assertEq(vault.balances(alice, address(usdc)), 5_000e6);
        assertEq(vault.balances(alice, address(weth)), 2e18);
    }
}
