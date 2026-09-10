import Foundation

let cameraPerformanceReportKind = "auki.camera-mesh.performance"
let cameraPerformanceSampleInterval: TimeInterval = 1

struct CameraPerformanceContext {
  let runtime: String
  let platform: String
  let domainID: String
  let localPeerID: String
  let columnCount: Int
}

struct CameraPerformanceSnapshot {
  let peerID: String
  let name: String
  let runtime: String
  let status: String
  let quality: String?
  let width: Int?
  let height: Int?
  let targetFPS: Int?
  let totalReceivedFrames: UInt64
  let totalRenderedFrames: UInt64
  let totalReceivedBytes: UInt64
  let receiveFPS: Double?
  let renderFPS: Double?
  let kibPerSecond: Double?
  let frameAgeMilliseconds: Double?
}

struct CameraPerformanceEvent: Codable, Equatable {
  let elapsedMs: Int
  let message: String
}

struct CameraPerformanceSample: Codable, Equatable {
  let elapsedMs: Int
  let columns: Int
  let status: String
  let quality: String?
  let width: Int?
  let height: Int?
  let targetFps: Int?
  let receivedFrames: UInt64
  let renderedFrames: UInt64
  let receivedBytes: UInt64
  let receiveFps: Double?
  let renderFps: Double?
  let kibPerSecond: Double?
  let frameAgeMs: Double?
}

struct CameraPerformanceNumberSummary: Codable, Equatable {
  let min: Double
  let average: Double
  let max: Double
  let p50: Double
  let p95: Double
}

struct CameraPerformancePeerSummary: Codable, Equatable {
  let sampleCount: Int
  let receivedFrames: UInt64
  let renderedFrames: UInt64
  let receivedBytes: UInt64
  let renderToReceiveRatio: Double?
  let receiveFps: CameraPerformanceNumberSummary?
  let renderFps: CameraPerformanceNumberSummary?
  let kibPerSecond: CameraPerformanceNumberSummary?
  let frameAgeMs: CameraPerformanceNumberSummary?
}

struct CameraPerformancePeerReport: Codable, Equatable {
  let peerId: String
  let name: String
  let runtime: String
  let samples: [CameraPerformanceSample]
  let summary: CameraPerformancePeerSummary
}

struct CameraPerformanceReport: Codable, Equatable {
  let schemaVersion: Int
  let kind: String
  let runtime: String
  let platform: String
  let domainId: String
  let localPeerId: String
  let startedAt: String
  let endedAt: String
  let durationMs: Int
  let sampleIntervalMs: Int
  let initialColumns: Int
  let finalColumns: Int
  let peers: [CameraPerformancePeerReport]
  let events: [CameraPerformanceEvent]

  func json() throws -> String {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
    return String(decoding: try encoder.encode(self), as: UTF8.self)
  }

  var filename: String {
    "camera-mesh-\(runtime)-\(startedAt.replacingOccurrences(of: ":", with: "-")).json"
  }
}

final class CameraPerformanceCapture {
  private struct PeerAccumulator {
    var name: String
    var runtime: String
    var lastReceivedFrames: UInt64?
    var lastRenderedFrames: UInt64?
    var lastReceivedBytes: UInt64?
    var receivedFrames: UInt64 = 0
    var renderedFrames: UInt64 = 0
    var receivedBytes: UInt64 = 0
    var samples: [CameraPerformanceSample] = []
  }

  let context: CameraPerformanceContext
  let startedAt: Date
  let startedAtMonotonic: TimeInterval
  private var peers: [String: PeerAccumulator] = [:]
  private var events: [CameraPerformanceEvent] = []

  init(
    context: CameraPerformanceContext,
    startedAt: Date = Date(),
    startedAtMonotonic: TimeInterval = ProcessInfo.processInfo.systemUptime
  ) {
    self.context = context
    self.startedAt = startedAt
    self.startedAtMonotonic = startedAtMonotonic
  }

  func sample(
    _ snapshots: [CameraPerformanceSnapshot],
    columnCount: Int,
    nowMonotonic: TimeInterval = ProcessInfo.processInfo.systemUptime
  ) {
    let elapsedMs = performanceElapsedMilliseconds(from: startedAtMonotonic, to: nowMonotonic)
    for snapshot in snapshots {
      var peer =
        peers[snapshot.peerID]
        ?? PeerAccumulator(name: snapshot.name, runtime: snapshot.runtime)
      peer.name = snapshot.name
      peer.runtime = snapshot.runtime
      peer.receivedFrames &+= performanceCounterDelta(
        current: snapshot.totalReceivedFrames,
        previous: peer.lastReceivedFrames
      )
      peer.renderedFrames &+= performanceCounterDelta(
        current: snapshot.totalRenderedFrames,
        previous: peer.lastRenderedFrames
      )
      peer.receivedBytes &+= performanceCounterDelta(
        current: snapshot.totalReceivedBytes,
        previous: peer.lastReceivedBytes
      )
      peer.lastReceivedFrames = snapshot.totalReceivedFrames
      peer.lastRenderedFrames = snapshot.totalRenderedFrames
      peer.lastReceivedBytes = snapshot.totalReceivedBytes

      let sample = CameraPerformanceSample(
        elapsedMs: elapsedMs,
        columns: columnCount,
        status: snapshot.status,
        quality: snapshot.quality,
        width: snapshot.width,
        height: snapshot.height,
        targetFps: snapshot.targetFPS,
        receivedFrames: peer.receivedFrames,
        renderedFrames: peer.renderedFrames,
        receivedBytes: peer.receivedBytes,
        receiveFps: performanceRound(snapshot.receiveFPS),
        renderFps: performanceRound(snapshot.renderFPS),
        kibPerSecond: performanceRound(snapshot.kibPerSecond),
        frameAgeMs: performanceRound(snapshot.frameAgeMilliseconds)
      )
      if peer.samples.last?.elapsedMs == elapsedMs {
        peer.samples[peer.samples.count - 1] = sample
      } else {
        peer.samples.append(sample)
      }
      peers[snapshot.peerID] = peer
    }
  }

  func recordEvent(
    _ message: String,
    nowMonotonic: TimeInterval = ProcessInfo.processInfo.systemUptime
  ) {
    events.append(
      CameraPerformanceEvent(
        elapsedMs: performanceElapsedMilliseconds(from: startedAtMonotonic, to: nowMonotonic),
        message: message
      ))
  }

  func finish(
    snapshots: [CameraPerformanceSnapshot],
    finalColumnCount: Int,
    endedAt: Date = Date(),
    endedAtMonotonic: TimeInterval = ProcessInfo.processInfo.systemUptime
  ) -> CameraPerformanceReport {
    sample(snapshots, columnCount: finalColumnCount, nowMonotonic: endedAtMonotonic)
    let peerReports = peers.keys.sorted().compactMap { peerID -> CameraPerformancePeerReport? in
      guard let peer = peers[peerID] else { return nil }
      return CameraPerformancePeerReport(
        peerId: peerID,
        name: peer.name,
        runtime: peer.runtime,
        samples: peer.samples,
        summary: performanceSummary(peer)
      )
    }
    return CameraPerformanceReport(
      schemaVersion: 1,
      kind: cameraPerformanceReportKind,
      runtime: context.runtime,
      platform: context.platform,
      domainId: context.domainID,
      localPeerId: context.localPeerID,
      startedAt: performanceISO8601(startedAt),
      endedAt: performanceISO8601(endedAt),
      durationMs: performanceElapsedMilliseconds(from: startedAtMonotonic, to: endedAtMonotonic),
      sampleIntervalMs: Int(cameraPerformanceSampleInterval * 1_000),
      initialColumns: context.columnCount,
      finalColumns: finalColumnCount,
      peers: peerReports,
      events: events
    )
  }

  private func performanceSummary(_ peer: PeerAccumulator) -> CameraPerformancePeerSummary {
    CameraPerformancePeerSummary(
      sampleCount: peer.samples.count,
      receivedFrames: peer.receivedFrames,
      renderedFrames: peer.renderedFrames,
      receivedBytes: peer.receivedBytes,
      renderToReceiveRatio: peer.receivedFrames > 0
        ? performanceRound(Double(peer.renderedFrames) / Double(peer.receivedFrames))
        : nil,
      receiveFps: performanceNumberSummary(peer.samples.compactMap(\.receiveFps)),
      renderFps: performanceNumberSummary(peer.samples.compactMap(\.renderFps)),
      kibPerSecond: performanceNumberSummary(peer.samples.compactMap(\.kibPerSecond)),
      frameAgeMs: performanceNumberSummary(peer.samples.compactMap(\.frameAgeMs))
    )
  }
}

private func performanceCounterDelta(current: UInt64, previous: UInt64?) -> UInt64 {
  guard let previous else { return 0 }
  return current >= previous ? current - previous : current
}

private func performanceElapsedMilliseconds(from start: TimeInterval, to end: TimeInterval) -> Int {
  max(0, Int(((end - start) * 1_000).rounded()))
}

private func performanceRound(_ value: Double?) -> Double? {
  guard let value, value.isFinite else { return nil }
  return (value * 1_000).rounded() / 1_000
}

private func performanceNumberSummary(
  _ values: [Double]
) -> CameraPerformanceNumberSummary? {
  let sorted = values.filter(\.isFinite).sorted()
  guard let minimum = sorted.first, let maximum = sorted.last else { return nil }
  return CameraPerformanceNumberSummary(
    min: performanceRound(minimum)!,
    average: performanceRound(sorted.reduce(0, +) / Double(sorted.count))!,
    max: performanceRound(maximum)!,
    p50: performanceRound(performancePercentile(sorted, percentile: 0.5))!,
    p95: performanceRound(performancePercentile(sorted, percentile: 0.95))!
  )
}

private func performancePercentile(_ sorted: [Double], percentile: Double) -> Double {
  let index = min(sorted.count - 1, max(0, Int(ceil(percentile * Double(sorted.count))) - 1))
  return sorted[index]
}

private func performanceISO8601(_ date: Date) -> String {
  let formatter = ISO8601DateFormatter()
  formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
  return formatter.string(from: date)
}
