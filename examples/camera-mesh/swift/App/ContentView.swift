import SwiftUI
import UIKit

struct ContentView: View {
  @ObservedObject var model: CameraMeshModel

  var body: some View {
    NavigationStack {
      Form {
        credentialsSection
        domainSection

        if !model.localCard.isEmpty {
          viewerSection
          discoverySection
          manualCardSection
        }

        if model.connection != nil || model.latestFrameImage != nil {
          cameraSection
        }

        if model.snapshotImage != nil {
          snapshotSection
        }

        if model.canStop {
          Section {
            Button("Stop viewer", role: .destructive) {
              Task { await model.stop() }
            }
          }
        }

        statusSection
      }
      .navigationTitle("Camera Mesh")
      .navigationBarTitleDisplayMode(.inline)
    }
  }

  private var credentialsSection: some View {
    Section("Auki User") {
      TextField("Email", text: $model.email)
        .textContentType(.username)
        .textInputAutocapitalization(.never)
        .autocorrectionDisabled()
        .disabled(model.phase != .signedOut)
      SecureField("Password", text: $model.password)
        .textContentType(.password)
        .disabled(model.phase != .signedOut)
      Button("Log in") { Task { await model.login() } }
        .disabled(!model.canLogin)
    }
  }

  @ViewBuilder
  private var domainSection: some View {
    if !model.domains.isEmpty {
      Section("Domain") {
        Picker("Domain", selection: $model.selectedDomainID) {
          ForEach(model.domains, id: \.id) { domain in
            Text(domain.name.map { "\($0) — \(domain.id)" } ?? domain.id)
              .tag(domain.id)
          }
        }
        Text("The viewer uses DDS discovery without advertising itself.")
          .font(.caption)
          .foregroundStyle(.secondary)
        Button("Start viewer") { Task { await model.start() } }
          .disabled(!model.canStart)
      }
    }
  }

  private var viewerSection: some View {
    Section("This viewer") {
      LabeledContent("Peer") {
        Text(shortPeerID(model.localPeerID))
          .font(.caption.monospaced())
          .textSelection(.enabled)
      }
      DisclosureGroup("Peer card") {
        Text(model.localCard)
          .font(.caption2.monospaced())
          .textSelection(.enabled)
      }
      Button {
        UIPasteboard.general.string = model.localCard
      } label: {
        Label("Copy peer card", systemImage: "doc.on.doc")
      }
    }
  }

  private var discoverySection: some View {
    Section("Discover publishers") {
      Text(
        "Discovery supplies route hints. The selected Peer ID and Domain are authenticated when Camera Mesh connects."
      )
      .font(.caption)
      .foregroundStyle(.secondary)

      Button {
        Task { await model.discover() }
      } label: {
        Label("Discover Stream publishers", systemImage: "dot.radiowaves.left.and.right")
      }
      .disabled(!model.canDiscover)

      if !model.discoveredCameras.isEmpty {
        Picker("Publisher", selection: $model.selectedCameraPeerID) {
          ForEach(model.discoveredCameras, id: \.peerID) { candidate in
            Text(shortPeerID(candidate.peerID)).tag(candidate.peerID)
          }
        }
        Button("Connect selected publisher") {
          Task { await model.connectSelectedCamera() }
        }
        .disabled(!model.canConnectDiscovered)
      } else {
        Text("No current publishers listed.")
          .font(.caption)
          .foregroundStyle(.secondary)
      }
    }
  }

  private var manualCardSection: some View {
    Section("Peer-card fallback") {
      Text("Paste a Web or native publisher card when discovery is unavailable.")
        .font(.caption)
        .foregroundStyle(.secondary)
      TextEditor(text: $model.remoteCard)
        .font(.caption.monospaced())
        .frame(minHeight: 112)
        .textInputAutocapitalization(.never)
        .autocorrectionDisabled()
      Button("Connect pasted publisher") {
        Task { await model.connectPastedCard() }
      }
      .disabled(!model.canConnectCard)

      if model.awaitingApproval {
        Label(
          "Access is waiting for the publisher to approve this exact viewer Peer ID.",
          systemImage: "person.badge.clock"
        )
        .font(.callout)
        .foregroundStyle(.orange)

        Button("Retry after approval") {
          Task { await model.retryConnection() }
        }
        .buttonStyle(.borderedProminent)
        .disabled(!model.canRetryConnection)
      } else if model.canRetryConnection {
        Button("Retry connection") {
          Task { await model.retryConnection() }
        }
      }
    }
  }

  private var cameraSection: some View {
    Section("Live camera") {
      if let connection = model.connection {
        VStack(alignment: .leading, spacing: 3) {
          Text(connection.name)
            .font(.headline)
          Text("\(connection.runtime) · \(shortPeerID(connection.peerID))")
            .font(.caption.monospaced())
            .foregroundStyle(.secondary)
            .textSelection(.enabled)
        }
      }

      CameraImage(image: model.latestFrameImage, placeholder: "Waiting for JPEG frames…")

      HStack {
        Label("\(model.frameCount) frames", systemImage: "photo.stack")
        Spacer()
        Text(model.latestSequence.map { "sequence \($0)" } ?? "no sequence")
          .foregroundStyle(.secondary)
      }
      .font(.caption.monospacedDigit())

      HStack {
        Button("Pause") { Task { await model.pause() } }
          .disabled(!model.canPause)
        Button("Resume") { Task { await model.resume() } }
          .disabled(!model.canResume)
        Spacer()
        Button {
          Task { await model.requestSnapshot() }
        } label: {
          if model.snapshotPending {
            ProgressView()
          } else {
            Label("Snapshot", systemImage: "camera")
          }
        }
        .disabled(!model.canRequestSnapshot)
      }

      Button("Disconnect camera", role: .destructive) {
        Task { await model.disconnect() }
      }
      .disabled(!model.canDisconnect)
    }
  }

  private var snapshotSection: some View {
    Section("Verified snapshot") {
      CameraImage(image: model.snapshotImage, placeholder: "No snapshot")
      Text(model.snapshotHash)
        .font(.caption2.monospaced())
        .textSelection(.enabled)
      Label(
        model.snapshotRelayed ? "Fetched through relay" : "Fetched directly",
        systemImage: model.snapshotRelayed ? "network" : "link"
      )
      .font(.caption)
      .foregroundStyle(.secondary)
    }
  }

  private var statusSection: some View {
    Section("Status — \(model.phase.rawValue)") {
      if model.phase == .authenticating || model.phase == .starting
        || model.phase == .discovering || model.phase == .connecting
        || model.phase == .controlling || model.phase == .disconnecting
        || model.phase == .stopping
      {
        ProgressView()
      }
      Text(model.log.isEmpty ? "Ready" : model.log)
        .font(.caption.monospaced())
        .textSelection(.enabled)
    }
  }

  private func shortPeerID(_ peerID: String) -> String {
    guard peerID.count > 20 else { return peerID }
    return "\(peerID.prefix(10))…\(peerID.suffix(8))"
  }
}

private struct CameraImage: View {
  let image: UIImage?
  let placeholder: String

  var body: some View {
    ZStack {
      RoundedRectangle(cornerRadius: 12)
        .fill(.black)
      if let image {
        Image(uiImage: image)
          .resizable()
          .scaledToFit()
      } else {
        VStack(spacing: 8) {
          Image(systemName: "video.slash")
            .font(.title2)
          Text(placeholder)
            .font(.caption)
        }
        .foregroundStyle(.white.opacity(0.7))
      }
    }
    .aspectRatio(16.0 / 9.0, contentMode: .fit)
    .clipShape(RoundedRectangle(cornerRadius: 12))
    .accessibilityLabel(image == nil ? placeholder : "Camera JPEG")
  }
}
