import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'
import { inspect } from 'node:util'

import { noise } from '@chainsafe/libp2p-noise'
import { yamux } from '@chainsafe/libp2p-yamux'
import { circuitRelayTransport } from '@libp2p/circuit-relay-v2'
import { identify } from '@libp2p/identify'
import { ping } from '@libp2p/ping'
import { webRTC } from '@libp2p/webrtc'
import { webSockets } from '@libp2p/websockets'
import { multiaddr } from '@multiformats/multiaddr'
import { createLibp2p } from 'libp2p'

const here = dirname(fileURLToPath(import.meta.url))
const targetFile = join(here, 'target-addr.txt')

function hasBrowserUsableRelayPath(addr) {
  const names = addr.getComponents().map((component) => component.name)
  const circuitIndex = names.indexOf('p2p-circuit')

  if (circuitIndex < 0) {
    return false
  }

  const relayPath = names.slice(0, circuitIndex)
  return relayPath.includes('ws') || relayPath.includes('wss')
}

function hasPrivateWebRtcTarget(addr) {
  const names = addr.getComponents().map((component) => component.name)
  const circuitIndex = names.indexOf('p2p-circuit')
  const webrtcIndex = names.indexOf('webrtc')

  return circuitIndex >= 0 && webrtcIndex > circuitIndex
}

function peerIdFromTarget(addr) {
  const p2pComponents = addr
    .getComponents()
    .filter((component) => component.name === 'p2p')
  const last = p2pComponents[p2pComponents.length - 1]

  if (last?.value == null) {
    return '<unknown>'
  }

  return last.value
}

function formatError(error) {
  if (error instanceof Error) {
    return error.message
  }

  return inspect(error, { depth: 3 })
}

async function readTargetAddr() {
  if (process.env.AUKI_RELAY_TARGET_ADDR != null && process.env.AUKI_RELAY_TARGET_ADDR !== '') {
    return process.env.AUKI_RELAY_TARGET_ADDR.trim()
  }

  return (await readFile(targetFile, 'utf8')).trim()
}

const targetAddrString = await readTargetAddr()
const targetAddr = multiaddr(targetAddrString)
const dialTimeoutMs = Number(process.env.AUKI_RELAY_DIAL_TIMEOUT_MS ?? 15_000)

if (!hasBrowserUsableRelayPath(targetAddr)) {
  throw new Error(
    `relay target must use a browser-usable /ws or /wss relay path before /p2p-circuit: ${targetAddrString}`
  )
}

if (!hasPrivateWebRtcTarget(targetAddr)) {
  throw new Error(
    `relay target must include /p2p-circuit/webrtc/p2p/<target>: ${targetAddrString}`
  )
}

const node = await createLibp2p({
  transports: [
    webSockets(),
    webRTC(),
    circuitRelayTransport()
  ],
  connectionEncrypters: [noise()],
  streamMuxers: [yamux()],
  services: {
    identify: identify(),
    ping: ping()
  }
})

try {
  console.error(`dialing ${targetAddrString}`)
  const connection = await node.dial(targetAddr, {
    signal: AbortSignal.timeout(dialTimeoutMs)
  })
  const targetPeerId = peerIdFromTarget(targetAddr)

  if (connection.remotePeer.toString() !== targetPeerId) {
    throw new Error(
      `connected to ${connection.remotePeer.toString()}, expected target peer ${targetPeerId}`
    )
  }

  console.log(`connected ${connection.remotePeer.toString()}`)
} catch (error) {
  throw new Error(`dial failed for ${targetAddrString}: ${formatError(error)}`)
} finally {
  await node.stop()
}
