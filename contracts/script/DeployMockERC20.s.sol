// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";
import {MockERC20Deployer} from "./Deploy.s.sol";

/// @notice Deploy standalone mock ERC-20 tokens and mint initial balances.
///
/// Useful when you already have a settlement contract deployed and just
/// need fresh tokens (e.g. after a testnet reset).
///
/// Usage:
///   forge script script/DeployMockERC20.s.sol:DeployMockTokens \
///     --rpc-url $SEPOLIA_RPC_URL \
///     --broadcast
///
/// Environment variables:
///   PRIVATE_KEY        - Deployer private key (required)
///   SETTLEMENT_ADDRESS - Existing settlement contract to approve (optional)
///   RECIPIENTS         - Comma-separated addresses to receive tokens (defaults to deployer)
///   MINT_AMOUNT_USDC   - USDC per recipient, 6 decimals (defaults to 1_000_000e6)
///   MINT_AMOUNT_WETH   - WETH per recipient, 18 decimals (defaults to 100e18)
contract DeployMockTokens is Script {
    function run() external {
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);

        uint256 usdcMint = vm.envOr("MINT_AMOUNT_USDC", uint256(1_000_000e6));
        uint256 wethMint = vm.envOr("MINT_AMOUNT_WETH", uint256(100e18));

        // Parse recipients — default to just the deployer
        address[] memory recipients = _parseRecipients(deployer);

        console.log("--- MockERC20 deployment ---");
        console.log("  Chain ID:    ", block.chainid);
        console.log("  Deployer:    ", deployer);
        console.log("  Recipients:  ", recipients.length);
        console.log("  USDC/user:   ", usdcMint);
        console.log("  WETH/user:   ", wethMint);

        vm.startBroadcast(deployerKey);

        MockERC20Deployer usdc = new MockERC20Deployer("USD Coin", "USDC", 6);
        MockERC20Deployer weth = new MockERC20Deployer("Wrapped Ether", "WETH", 18);

        for (uint256 i = 0; i < recipients.length; i++) {
            usdc.mint(recipients[i], usdcMint);
            weth.mint(recipients[i], wethMint);
        }

        vm.stopBroadcast();

        console.log("  USDC:        ", address(usdc));
        console.log("  WETH:        ", address(weth));
        console.log("---");
    }

    function _parseRecipients(address fallback_) internal view returns (address[] memory) {
        try vm.envString("RECIPIENTS") returns (string memory raw) {
            // Split comma-separated addresses
            string[] memory parts = vm.split(raw, ",");
            address[] memory addrs = new address[](parts.length);
            for (uint256 i = 0; i < parts.length; i++) {
                addrs[i] = vm.parseAddress(vm.trim(parts[i]));
            }
            return addrs;
        } catch {
            address[] memory single = new address[](1);
            single[0] = fallback_;
            return single;
        }
    }
}
