// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";
import {IntentXSettlement} from "../src/IntentXSettlement.sol";

/// @notice Deploy IntentXSettlement on any EVM chain.
///
/// Usage (mainnet, with Etherscan verification):
///   forge script script/Deploy.s.sol:DeploySettlement \
///     --rpc-url $ETH_RPC_URL \
///     --broadcast \
///     --verify \
///     --etherscan-api-key $ETHERSCAN_API_KEY
///
/// Usage (testnet, no verification):
///   forge script script/Deploy.s.sol:DeploySettlement \
///     --rpc-url $SEPOLIA_RPC_URL \
///     --broadcast
///
/// Environment variables:
///   PRIVATE_KEY     - Deployer private key (required)
///   AUTHORITY       - Backend signer address (defaults to deployer)
///   FEE_RECIPIENT   - Fee collection address (defaults to deployer)
///   FEE_BPS         - Fee in basis points, max 5000 (defaults to 10 = 0.1%)
contract DeploySettlement is Script {
    function run() external returns (IntentXSettlement) {
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);

        address authority = vm.envOr("AUTHORITY", deployer);
        address feeRecipient = vm.envOr("FEE_RECIPIENT", deployer);
        uint16 feeBps = uint16(vm.envOr("FEE_BPS", uint256(10)));

        console.log("--- IntentXSettlement deployment ---");
        console.log("  Chain ID:      ", block.chainid);
        console.log("  Deployer:      ", deployer);
        console.log("  Authority:     ", authority);
        console.log("  Fee recipient: ", feeRecipient);
        console.log("  Fee bps:       ", feeBps);

        vm.startBroadcast(deployerKey);
        IntentXSettlement settlement = deploy(authority, feeRecipient, feeBps);
        vm.stopBroadcast();

        console.log("  Settlement:    ", address(settlement));
        console.log("---");
        return settlement;
    }

    /// @dev Core deployment logic, callable from tests without env vars.
    function deploy(
        address authority,
        address feeRecipient,
        uint16 feeBps
    ) public returns (IntentXSettlement) {
        return new IntentXSettlement(authority, feeRecipient, feeBps);
    }
}

/// @notice Full testnet deployment: settlement contract + mock tokens + initial balances.
///
/// Usage:
///   forge script script/Deploy.s.sol:DeployTestnet \
///     --rpc-url $SEPOLIA_RPC_URL \
///     --broadcast \
///     --verify \
///     --etherscan-api-key $ETHERSCAN_API_KEY
///
/// Environment variables (all optional except PRIVATE_KEY):
///   PRIVATE_KEY     - Deployer private key (required)
///   AUTHORITY       - Backend signer address (defaults to deployer)
///   FEE_RECIPIENT   - Fee collection address (defaults to deployer)
///   FEE_BPS         - Fee in basis points (defaults to 100 = 1%)
///   ALICE           - Test user to receive initial tokens (defaults to deployer)
///   BOB             - Second test user (defaults to deployer)
///   MINT_AMOUNT_USDC - USDC mint amount per user, 6 decimals (defaults to 1_000_000e6)
///   MINT_AMOUNT_WETH - WETH mint amount per user, 18 decimals (defaults to 100e18)
contract DeployTestnet is Script {
    function run() external {
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);

        address authority = vm.envOr("AUTHORITY", deployer);
        address feeRecipient = vm.envOr("FEE_RECIPIENT", deployer);
        uint16 feeBps = uint16(vm.envOr("FEE_BPS", uint256(100)));

        address alice = vm.envOr("ALICE", deployer);
        address bob = vm.envOr("BOB", deployer);
        uint256 usdcMint = vm.envOr("MINT_AMOUNT_USDC", uint256(1_000_000e6));
        uint256 wethMint = vm.envOr("MINT_AMOUNT_WETH", uint256(100e18));

        console.log("--- IntentX testnet deployment ---");
        console.log("  Chain ID:      ", block.chainid);
        console.log("  Deployer:      ", deployer);

        vm.startBroadcast(deployerKey);

        (
            IntentXSettlement settlement,
            MockERC20Deployer usdc,
            MockERC20Deployer weth
        ) = deployAll(authority, feeRecipient, feeBps, alice, bob, usdcMint, wethMint);

        vm.stopBroadcast();

        console.log("  USDC:          ", address(usdc));
        console.log("  WETH:          ", address(weth));
        console.log("  Settlement:    ", address(settlement));
        console.log("---");
    }

    /// @dev Core deployment logic, callable from tests without env vars.
    function deployAll(
        address authority,
        address feeRecipient,
        uint16 feeBps,
        address alice,
        address bob,
        uint256 usdcMint,
        uint256 wethMint
    )
        public
        returns (
            IntentXSettlement settlement,
            MockERC20Deployer usdc,
            MockERC20Deployer weth
        )
    {
        usdc = new MockERC20Deployer("USD Coin", "USDC", 6);
        weth = new MockERC20Deployer("Wrapped Ether", "WETH", 18);
        settlement = new IntentXSettlement(authority, feeRecipient, feeBps);

        usdc.mint(alice, usdcMint);
        usdc.mint(bob, usdcMint);
        weth.mint(alice, wethMint);
        weth.mint(bob, wethMint);
    }
}

/// @dev Minimal ERC-20 for deployment scripts. Same interface as
///      test/mocks/MockERC20.sol but independent of the test directory
///      so scripts can deploy it without importing test code.
contract MockERC20Deployer {
    string public name;
    string public symbol;
    uint8 public decimals;
    uint256 public totalSupply;

    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);

    constructor(string memory name_, string memory symbol_, uint8 decimals_) {
        name = name_;
        symbol = symbol_;
        decimals = decimals_;
    }

    function mint(address to, uint256 amount) external {
        totalSupply += amount;
        balanceOf[to] += amount;
        emit Transfer(address(0), to, amount);
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        emit Approval(msg.sender, spender, amount);
        return true;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        require(balanceOf[msg.sender] >= amount, "insufficient balance");
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        emit Transfer(msg.sender, to, amount);
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        require(balanceOf[from] >= amount, "insufficient balance");
        require(allowance[from][msg.sender] >= amount, "insufficient allowance");
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        emit Transfer(from, to, amount);
        return true;
    }
}
