import Foundation
import auki_network

let seed = Data(repeating: 3, count: 32)
let expectedPeerId = "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"

let peerId = try peerIdFromWalletSeed(seed: seed)
precondition(peerId == expectedPeerId, "unexpected generated peer id")

let runtime = try AukiNetworkRuntime.spawn(config: BindingSwarmConfig(
    walletSeed: seed,
    listenMultiaddrs: ["/ip4/127.0.0.1/tcp/0"],
    agentVersion: "auki-network-swift-smoke/0.1",
    allowedPeers: [],
    heartbeatClockId: nil,
    heartbeatClockHashHex: nil
))

precondition(!runtime.localPeerId().isEmpty, "runtime peer id is empty")
try runtime.shutdown()
