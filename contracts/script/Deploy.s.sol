// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Script, console} from "forge-std/Script.sol";
import {IntentXSettlement} from "../src/IntentXSettlement.sol";

/// @notice Deploy the IntentXSettlement contract.
///
/// Usage:
///   forge script script/Deploy.s.sol --rpc-url $RPC_URL --broadcast --verify
///
/// Environment variables:
///   AUTHORITY       - Backend signer address (defaults to deployer)
///   FEE_RECIPIENT   - Fee collection address (defaults to deployer)
///   FEE_BPS         - Fee in basis points (defaults to 100 = 1%)
contract DeploySettlement is Script {
    function run() external {
        uint256 deployerKey = vm.envUint("PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);

        address authority = vm.envOr("AUTHORITY", deployer);
        address feeRecipient = vm.envOr("FEE_RECIPIENT", deployer);
        uint16 feeBps = uint16(vm.envOr("FEE_BPS", uint256(100)));

        console.log("Deployer:", deployer);
        console.log("Authority:", authority);
        console.log("Fee recipient:", feeRecipient);
        console.log("Fee bps:", feeBps);

        vm.startBroadcast(deployerKey);

        IntentXSettlement settlement = new IntentXSettlement(
            authority,
            feeRecipient,
            feeBps
        );

        vm.stopBroadcast();

        console.log("IntentXSettlement deployed at:", address(settlement));
    }
}
