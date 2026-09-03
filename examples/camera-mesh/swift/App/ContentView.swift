import AukiCameraMesh
import SwiftUI
import UIKit

struct ContentView: View {
  @ObservedObject var model: CameraMeshModel

  var body: some View {
    Group {
      if model.localCard.isEmpty {
        CameraMeshSetupView(model: model)
      } else if model.selectedRole == .viewer {
        CameraMonitorView(model: model)
      } else {
        CameraPublisherView(model: model)
      }
    }
    .preferredColorScheme(.dark)
    .tint(CameraMeshStyle.accent)
  }
}

private enum CameraMeshStyle {
  static let background = Color(red: 0.018, green: 0.027, blue: 0.035)
  static let surface = Color(red: 0.045, green: 0.067, blue: 0.082)
  static let raised = Color(red: 0.075, green: 0.105, blue: 0.125)
  static let line = Color.white.opacity(0.12)
  static let text = Color(red: 0.94, green: 0.97, blue: 0.97)
  static let muted = Color(red: 0.60, green: 0.67, blue: 0.69)
  static let accent = Color(red: 0.35, green: 0.89, blue: 0.68)
  static let warning = Color(red: 0.96, green: 0.74, blue: 0.36)
  static let danger = Color(red: 1.0, green: 0.49, blue: 0.52)
}

private struct CameraMeshSetupView: View {
  @ObservedObject var model: CameraMeshModel

  private var isBusy: Bool {
    model.phase == .authenticating || model.phase == .starting
  }

  var body: some View {
    ZStack {
      CameraMeshStyle.background.ignoresSafeArea()
      RadialGradient(
        colors: [CameraMeshStyle.accent.opacity(0.11), .clear],
        center: .topLeading,
        startRadius: 0,
        endRadius: 560
      )
      .ignoresSafeArea()

      ScrollView {
        VStack(alignment: .leading, spacing: 28) {
          CameraMeshBrand(showName: true)

          VStack(alignment: .leading, spacing: 8) {
            Text("AUTHENTICATED CAMERA MONITORING")
              .font(.caption.weight(.bold))
              .tracking(2)
              .foregroundStyle(CameraMeshStyle.accent)
            Text("Your camera wall, without the control-room clutter.")
              .font(.system(size: 38, weight: .bold, design: .rounded))
              .foregroundStyle(CameraMeshStyle.text)
            Text(
              "Sign in, choose a Domain, then monitor or publish cameras through the Auki peer network."
            )
            .font(.body)
            .foregroundStyle(CameraMeshStyle.muted)
          }

          VStack(alignment: .leading, spacing: 18) {
            if model.phase == .signedOut || model.phase == .authenticating {
              setupCredentials
            } else {
              setupDomainAndRole
            }

            if isBusy {
              HStack(spacing: 10) {
                ProgressView()
                Text(model.phase.rawValue)
                  .font(.callout.weight(.semibold))
              }
              .foregroundStyle(CameraMeshStyle.muted)
            }

            if !model.log.isEmpty {
              Text(model.log)
                .font(.caption.monospaced())
                .foregroundStyle(CameraMeshStyle.muted)
                .lineLimit(5)
                .textSelection(.enabled)
            }
          }
          .padding(20)
          .background(CameraMeshStyle.surface)
          .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
          .overlay {
            RoundedRectangle(cornerRadius: 18, style: .continuous)
              .stroke(CameraMeshStyle.line)
          }

          Text("Peer identities and credentials are ephemeral in this example.")
            .font(.caption)
            .foregroundStyle(CameraMeshStyle.muted.opacity(0.75))
        }
        .frame(maxWidth: 680)
        .padding(.horizontal, 22)
        .padding(.vertical, 28)
        .frame(maxWidth: .infinity)
      }
    }
  }

  private var setupCredentials: some View {
    VStack(alignment: .leading, spacing: 14) {
      Text("Sign in")
        .font(.title2.bold())
      TextField("Email", text: $model.email)
        .textContentType(.username)
        .textInputAutocapitalization(.never)
        .autocorrectionDisabled()
        .cameraMeshField()
        .disabled(model.phase != .signedOut)
      SecureField("Password", text: $model.password)
        .textContentType(.password)
        .cameraMeshField()
        .disabled(model.phase != .signedOut)
      Button {
        Task { await model.login() }
      } label: {
        Text("Continue")
          .frame(maxWidth: .infinity)
      }
      .buttonStyle(CameraMeshPrimaryButtonStyle())
      .disabled(!model.canLogin)
    }
  }

  private var setupDomainAndRole: some View {
    VStack(alignment: .leading, spacing: 14) {
      Text("Open Camera Mesh")
        .font(.title2.bold())
      Picker("Domain", selection: $model.selectedDomainID) {
        ForEach(model.domains, id: \.id) { domain in
          Text(domain.name ?? shortCameraPeerID(domain.id)).tag(domain.id)
        }
      }
      .pickerStyle(.menu)
      .cameraMeshField()

      Picker("Mode", selection: $model.selectedRole) {
        ForEach(CameraMeshRole.allCases) { role in
          Text(role.title).tag(role)
        }
      }
      .pickerStyle(.segmented)

      Text(roleExplanation)
        .font(.caption)
        .foregroundStyle(CameraMeshStyle.muted)

      Button {
        Task { await model.start() }
      } label: {
        Text("Start \(model.selectedRole.title.lowercased())")
          .frame(maxWidth: .infinity)
      }
      .buttonStyle(CameraMeshPrimaryButtonStyle())
      .disabled(!model.canStart)
    }
  }

  private var roleExplanation: String {
    switch model.selectedRole {
    case .viewer:
      "Discover cameras without advertising this viewer."
    case .publisher:
      "Publish this iPhone camera and approve viewers explicitly."
    }
  }
}

private struct CameraMonitorView: View {
  @ObservedObject var model: CameraMeshModel
  @State private var requestedColumns = 2
  @State private var presentedSheet: MonitorSheet?

  private enum MonitorSheet: Identifiable {
    case addCamera
    case session
    case snapshot(String)

    var id: String {
      switch self {
      case .addCamera: "add-camera"
      case .session: "session"
      case .snapshot(let peerID): "snapshot-\(peerID)"
      }
    }
  }

  var body: some View {
    GeometryReader { geometry in
      let compact = UIDevice.current.userInterfaceIdiom == .phone || geometry.size.width < 720
      let columns = effectiveCameraColumnCount(
        requested: requestedColumns,
        compact: compact
      )

      VStack(spacing: 0) {
        monitorHeader(compact: compact, effectiveColumns: columns)
        Divider().overlay(CameraMeshStyle.line)
        cameraWall(columns: columns, compact: compact)
      }
      .background(CameraMeshStyle.background)
    }
    .sheet(item: $presentedSheet) { sheet in
      switch sheet {
      case .addCamera:
        AddCameraSheet(model: model) { presentedSheet = nil }
      case .session:
        CameraSessionSheet(model: model) { presentedSheet = nil }
      case .snapshot(let peerID):
        if let tile = model.cameraTiles.first(where: { $0.peerID == peerID }) {
          CameraSnapshotSheet(tile: tile) { presentedSheet = nil }
        }
      }
    }
  }

  @ViewBuilder
  private func monitorHeader(compact: Bool, effectiveColumns: Int) -> some View {
    if compact {
      VStack(spacing: 8) {
        monitorIdentity
        monitorControls(compact: true, effectiveColumns: effectiveColumns)
      }
      .padding(.horizontal, 12)
      .padding(.vertical, 8)
      .background(CameraMeshStyle.surface.opacity(0.96))
    } else {
      HStack(spacing: 18) {
        monitorIdentity
        Spacer(minLength: 12)
        monitorControls(compact: false, effectiveColumns: effectiveColumns)
      }
      .padding(.horizontal, 14)
      .padding(.vertical, 10)
      .background(CameraMeshStyle.surface.opacity(0.96))
    }
  }

  private var monitorIdentity: some View {
    HStack(spacing: 10) {
      CameraMeshBrand(showName: true)
      Spacer(minLength: 8)
      HStack(spacing: 6) {
        Circle()
          .fill(model.liveCameraCount > 0 ? CameraMeshStyle.accent : CameraMeshStyle.muted)
          .frame(width: 8, height: 8)
        Text(selectedDomainName)
          .font(.caption.weight(.bold))
          .lineLimit(1)
        Text("· \(model.liveCameraCount) live")
          .font(.caption.monospacedDigit())
          .foregroundStyle(CameraMeshStyle.muted)
      }
      Button {
        presentedSheet = .session
      } label: {
        Image(systemName: "ellipsis")
          .frame(width: 44, height: 36)
      }
      .buttonStyle(.plain)
      .accessibilityLabel("Session details")
    }
  }

  private func monitorControls(compact: Bool, effectiveColumns: Int) -> some View {
    HStack(spacing: 8) {
      HStack(spacing: 2) {
        ForEach(compact ? [1, 2] : [1, 2, 3, 4], id: \.self) { count in
          Button {
            requestedColumns = count
            if count == 1, model.focusedCameraPeerID == nil {
              model.focusedCameraPeerID = model.cameraTiles.first?.peerID
            }
          } label: {
            Text("\(count)")
              .font(.caption.monospacedDigit().weight(.bold))
              .frame(maxWidth: .infinity, minHeight: 38)
              .padding(.horizontal, compact ? 8 : 12)
              .background(
                effectiveColumns == count ? CameraMeshStyle.raised : Color.clear
              )
              .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
          }
          .buttonStyle(.plain)
          .accessibilityLabel("\(count) camera column\(count == 1 ? "" : "s")")
        }
      }
      .padding(3)
      .background(Color.black.opacity(0.26))
      .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
      .overlay {
        RoundedRectangle(cornerRadius: 10, style: .continuous)
          .stroke(CameraMeshStyle.line)
      }

      if effectiveColumns == 1, model.cameraTiles.count > 1 {
        HStack(spacing: 0) {
          Button {
            model.moveFocus(by: -1)
          } label: {
            Image(systemName: "chevron.left").frame(width: 40, height: 40)
          }
          Button {
            model.moveFocus(by: 1)
          } label: {
            Image(systemName: "chevron.right").frame(width: 40, height: 40)
          }
        }
        .buttonStyle(.plain)
      }

      Button {
        presentedSheet = .addCamera
      } label: {
        Group {
          if compact {
            Image(systemName: "plus")
          } else {
            Label("Add camera", systemImage: "plus")
          }
        }
        .font(.callout.weight(.bold))
        .frame(minWidth: 44, minHeight: 44)
        .padding(.horizontal, compact ? 0 : 10)
        .background(CameraMeshStyle.accent.opacity(0.85))
        .foregroundStyle(Color.black.opacity(0.78))
        .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
      }
      .buttonStyle(.plain)
      .disabled(model.remainingCameraSlots == 0)
    }
  }

  @ViewBuilder
  private func cameraWall(columns: Int, compact: Bool) -> some View {
    if columns == 1 {
      if let focusedTile {
        CameraTileView(
          tile: focusedTile,
          number: focusedCameraNumber,
          compact: false,
          focused: true,
          onFocus: { model.focusCamera(peerID: focusedTile.peerID) },
          onRetry: { Task { await model.retryCamera(peerID: focusedTile.peerID) } },
          onPause: { Task { await model.pauseCamera(peerID: focusedTile.peerID) } },
          onResume: { Task { await model.resumeCamera(peerID: focusedTile.peerID) } },
          onSnapshot: { Task { await model.requestSnapshot(peerID: focusedTile.peerID) } },
          onViewSnapshot: { presentedSheet = .snapshot(focusedTile.peerID) },
          onRemove: { Task { await model.removeCamera(peerID: focusedTile.peerID) } }
        )
        .padding(5)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
      } else {
        EmptyCameraWall { presentedSheet = .addCamera }
      }
    } else {
      ScrollView {
        LazyVGrid(
          columns: Array(
            repeating: GridItem(.flexible(minimum: 0), spacing: 5),
            count: columns
          ),
          spacing: 5
        ) {
          ForEach(Array(model.cameraTiles.enumerated()), id: \.element.peerID) { index, tile in
            CameraTileView(
              tile: tile,
              number: index + 1,
              compact: compact,
              focused: false,
              onFocus: {
                model.focusCamera(peerID: tile.peerID)
                requestedColumns = 1
              },
              onRetry: { Task { await model.retryCamera(peerID: tile.peerID) } },
              onPause: { Task { await model.pauseCamera(peerID: tile.peerID) } },
              onResume: { Task { await model.resumeCamera(peerID: tile.peerID) } },
              onSnapshot: { Task { await model.requestSnapshot(peerID: tile.peerID) } },
              onViewSnapshot: { presentedSheet = .snapshot(tile.peerID) },
              onRemove: { Task { await model.removeCamera(peerID: tile.peerID) } }
            )
            .aspectRatio(16.0 / 9.0, contentMode: .fit)
          }

          if model.remainingCameraSlots > 0 {
            EmptyCameraTile { presentedSheet = .addCamera }
              .aspectRatio(16.0 / 9.0, contentMode: .fit)
          }
        }
        .padding(5)
      }
      .background(Color.black.opacity(0.4))
    }
  }

  private var focusedTile: CameraTile? {
    if let peerID = model.focusedCameraPeerID,
      let tile = model.cameraTiles.first(where: { $0.peerID == peerID })
    {
      return tile
    }
    return model.cameraTiles.first
  }

  private var focusedCameraNumber: Int {
    guard let peerID = focusedTile?.peerID,
      let index = model.cameraTiles.firstIndex(where: { $0.peerID == peerID })
    else { return 1 }
    return index + 1
  }

  private var selectedDomainName: String {
    model.domains.first(where: { $0.id == model.selectedDomainID })?.name
      ?? shortCameraPeerID(model.selectedDomainID)
  }
}

private struct CameraTileView: View {
  @ObservedObject var tile: CameraTile
  let number: Int
  let compact: Bool
  let focused: Bool
  let onFocus: () -> Void
  let onRetry: () -> Void
  let onPause: () -> Void
  let onResume: () -> Void
  let onSnapshot: () -> Void
  let onViewSnapshot: () -> Void
  let onRemove: () -> Void

  var body: some View {
    ZStack {
      Color.black

      if let image = tile.image {
        Image(uiImage: image)
          .resizable()
          .scaledToFit()
          .frame(maxWidth: .infinity, maxHeight: .infinity)
          .opacity(tile.status == .live ? 1 : 0.38)
      }

      LinearGradient(
        stops: [
          .init(color: .black.opacity(0.72), location: 0),
          .init(color: .clear, location: 0.30),
          .init(color: .clear, location: 0.62),
          .init(color: .black.opacity(0.82), location: 1),
        ],
        startPoint: .top,
        endPoint: .bottom
      )

      VStack(spacing: 0) {
        tileHeader
        Spacer(minLength: 0)
        if tile.image == nil || tile.status != .live {
          tileState
        }
        Spacer(minLength: 0)
        tileFooter
      }
      .padding(focused ? 14 : 8)
    }
    .contentShape(Rectangle())
    .onTapGesture { onFocus() }
    .clipShape(RoundedRectangle(cornerRadius: focused ? 10 : 5, style: .continuous))
    .overlay {
      RoundedRectangle(cornerRadius: focused ? 10 : 5, style: .continuous)
        .stroke(borderColor, lineWidth: 1)
    }
    .accessibilityElement(children: .contain)
    .accessibilityLabel("\(tile.name), \(statusLabel)")
  }

  private var tileHeader: some View {
    HStack(spacing: 7) {
      Text("CAM \(String(format: "%02d", number))")
        .font(.system(size: focused ? 12 : 9, design: .monospaced))
        .foregroundStyle(CameraMeshStyle.muted)
      Text(tile.name)
        .font(.system(size: focused ? 16 : 11, weight: .bold))
        .foregroundStyle(CameraMeshStyle.text)
        .lineLimit(1)
      Spacer(minLength: 5)
      Circle()
        .fill(statusColor)
        .frame(width: focused ? 9 : 7, height: focused ? 9 : 7)
      Text(statusLabel.uppercased())
        .font(.system(size: focused ? 11 : 8, weight: .semibold, design: .monospaced))
        .foregroundStyle(CameraMeshStyle.text.opacity(0.9))
    }
  }

  private var tileState: some View {
    VStack(spacing: focused ? 12 : 6) {
      if tile.status == .connecting || tile.status == .waiting {
        ProgressView()
          .tint(CameraMeshStyle.warning)
      } else {
        Image(systemName: tile.status == .awaitingApproval ? "person.badge.clock" : "video.slash")
          .font(focused ? .title : .body)
          .foregroundStyle(statusColor)
      }
      Text(stateHeading)
        .font(.system(size: focused ? 18 : 11, weight: .bold))
        .foregroundStyle(CameraMeshStyle.text)
      if focused || !compact {
        Text(tile.message)
          .font(.system(size: focused ? 13 : 9))
          .foregroundStyle(CameraMeshStyle.muted)
          .multilineTextAlignment(.center)
          .lineLimit(focused ? 4 : 2)
          .frame(maxWidth: focused ? 420 : 240)
      }
      if canRetry {
        Button("Retry", action: onRetry)
          .font(.caption.bold())
          .buttonStyle(.bordered)
          .controlSize(focused ? .regular : .small)
      }
    }
  }

  private var tileFooter: some View {
    HStack(alignment: .bottom, spacing: 8) {
      if focused || !compact {
        VStack(alignment: .leading, spacing: 2) {
          TimelineView(.periodic(from: .now, by: 1)) { context in
            Text(frameAge(at: context.date))
          }
          Text("\(tile.frameCount) frames · \(shortCameraPeerID(tile.peerID))")
            .lineLimit(1)
        }
        .font(.system(size: focused ? 11 : 8, design: .monospaced))
        .foregroundStyle(CameraMeshStyle.muted)
      }
      Spacer(minLength: 0)
      Menu {
        if tile.connectionID != nil {
          Button(tile.paused ? "Resume camera" : "Pause camera") {
            tile.paused ? onResume() : onPause()
          }
          Button("Request verified snapshot", action: onSnapshot)
            .disabled(tile.snapshotPending)
        }
        if tile.snapshotImage != nil {
          Button("View last snapshot", action: onViewSnapshot)
        }
        if canRetry {
          Button("Retry connection", action: onRetry)
        }
        Button("Focus camera", action: onFocus)
        Divider()
        Button("Remove camera", role: .destructive, action: onRemove)
      } label: {
        Image(systemName: "ellipsis")
          .font(.body.weight(.bold))
          .frame(width: 44, height: 40)
          .background(Color.black.opacity(0.58))
          .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
      }
      .disabled(tile.controlPending)
    }
  }

  private var canRetry: Bool {
    tile.status == .awaitingApproval || tile.status == .ended || tile.status == .error
  }

  private var statusLabel: String {
    if tile.paused { return "paused" }
    return switch tile.status {
    case .connecting: "connecting"
    case .waiting: "waiting"
    case .live: "live"
    case .awaitingApproval: "approval"
    case .ended: "offline"
    case .error: "error"
    }
  }

  private var statusColor: Color {
    switch tile.status {
    case .live: tile.paused ? CameraMeshStyle.warning : CameraMeshStyle.accent
    case .connecting, .waiting, .awaitingApproval: CameraMeshStyle.warning
    case .ended, .error: CameraMeshStyle.danger
    }
  }

  private var borderColor: Color {
    switch tile.status {
    case .live: CameraMeshStyle.accent.opacity(0.28)
    case .connecting, .waiting, .awaitingApproval: CameraMeshStyle.warning.opacity(0.40)
    case .ended, .error: CameraMeshStyle.danger.opacity(0.38)
    }
  }

  private var stateHeading: String {
    switch tile.status {
    case .connecting: "Connecting"
    case .waiting: "Waiting for video"
    case .live: tile.paused ? "Camera paused" : "Live feed"
    case .awaitingApproval: "Approval required"
    case .ended: "Camera offline"
    case .error: "Could not connect"
    }
  }

  private func frameAge(at date: Date) -> String {
    guard let latestFrameAt = tile.latestFrameAt else { return "NO SIGNAL" }
    let age = max(0, date.timeIntervalSince(latestFrameAt))
    if age < 1 { return "LIVE · <1S" }
    return "LIVE · \(Int(age))S"
  }
}

private struct EmptyCameraWall: View {
  let action: () -> Void

  var body: some View {
    ZStack {
      CameraMeshStyle.background
      EmptyCameraTile(action: action)
        .frame(maxWidth: 520)
        .aspectRatio(16.0 / 9.0, contentMode: .fit)
        .padding()
    }
  }
}

private struct EmptyCameraTile: View {
  let action: () -> Void

  var body: some View {
    Button(action: action) {
      VStack(spacing: 8) {
        Image(systemName: "plus")
          .font(.title2.weight(.light))
        Text("Add camera")
          .font(.caption.weight(.semibold))
      }
      .frame(maxWidth: .infinity, maxHeight: .infinity)
      .foregroundStyle(CameraMeshStyle.muted)
      .background(CameraMeshStyle.surface.opacity(0.5))
      .overlay {
        RoundedRectangle(cornerRadius: 5, style: .continuous)
          .stroke(CameraMeshStyle.line, style: StrokeStyle(lineWidth: 1, dash: [6]))
      }
    }
    .buttonStyle(.plain)
  }
}

private struct AddCameraSheet: View {
  @ObservedObject var model: CameraMeshModel
  let dismiss: () -> Void

  var body: some View {
    NavigationStack {
      List {
        Section {
          HStack {
            Button {
              Task { await model.discover() }
            } label: {
              Label("Refresh", systemImage: "arrow.clockwise")
            }
            .buttonStyle(.borderless)
            .disabled(!model.canDiscover)

            Spacer()

            Button {
              Task { await model.discoverAndAddAllCameras() }
            } label: {
              if model.addingAllCameras {
                ProgressView()
                  .frame(minWidth: 72)
              } else {
                Label("Add all", systemImage: "rectangle.stack.badge.plus")
              }
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.regular)
            .disabled(!model.canAddAllCameras)
          }
        } header: {
          Text("Discover publishers")
        } footer: {
          Text(
            "\(model.cameraTiles.count) of \(CameraMeshContract.maximumViewerConnections) wall slots used."
          )
        }

        Section {
          if model.discoveredCameras.isEmpty {
            ContentUnavailableView(
              "No cameras found",
              systemImage: "video.slash",
              description: Text("Refresh discovery or paste a peer card below.")
            )
          } else {
            ForEach(model.discoveredCameras) { candidate in
              let existing = model.cameraTiles.first(where: { $0.peerID == candidate.peerID })
              HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 3) {
                  Text(existing?.name ?? "Camera publisher")
                    .font(.callout.weight(.semibold))
                  Text(shortCameraPeerID(candidate.peerID))
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                }
                Spacer()
                if let existing {
                  if existing.status == .error || existing.status == .ended
                    || existing.status == .awaitingApproval
                  {
                    Button("Retry") {
                      Task { await model.retryCamera(peerID: candidate.peerID) }
                    }
                  } else {
                    Text(existing.status == .live ? "On wall" : "Adding…")
                      .font(.caption.weight(.semibold))
                      .foregroundStyle(.secondary)
                  }
                } else {
                  Button("Add") {
                    model.selectedCameraPeerID = candidate.peerID
                    Task { await model.connectSelectedCamera() }
                  }
                  .disabled(model.remainingCameraSlots == 0)
                }
              }
            }
          }
        }

        Section("Peer-card fallback") {
          TextEditor(text: $model.remoteCard)
            .font(.caption.monospaced())
            .frame(minHeight: 112)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
          Button("Add pasted camera") {
            Task { await model.connectPastedCard() }
          }
          .disabled(!model.canConnectCard)
        }
      }
      .navigationTitle("Add camera")
      .navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .confirmationAction) {
          Button("Done", action: dismiss)
        }
      }
    }
  }

}

private struct CameraSessionSheet: View {
  @ObservedObject var model: CameraMeshModel
  let dismiss: () -> Void

  var body: some View {
    NavigationStack {
      List {
        Section("Viewer") {
          LabeledContent("Status", value: model.phase.rawValue)
          LabeledContent(
            "Cameras", value: "\(model.liveCameraCount) live / \(model.cameraTiles.count) on wall")
          VStack(alignment: .leading, spacing: 5) {
            Text("Peer ID").font(.caption).foregroundStyle(.secondary)
            Text(model.localPeerID)
              .font(.caption2.monospaced())
              .textSelection(.enabled)
          }
          Button("Copy Peer ID") { UIPasteboard.general.string = model.localPeerID }
          DisclosureGroup("Peer card") {
            Text(model.localCard)
              .font(.caption2.monospaced())
              .textSelection(.enabled)
            Button("Copy peer card") { UIPasteboard.general.string = model.localCard }
          }
        }

        Section("Runtime log") {
          Text(model.log.isEmpty ? "Ready" : model.log)
            .font(.caption.monospaced())
            .textSelection(.enabled)
        }

        Section {
          Button("Stop viewer", role: .destructive) {
            dismiss()
            Task { await model.stop() }
          }
        }
      }
      .navigationTitle("Camera Mesh")
      .navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .confirmationAction) {
          Button("Done", action: dismiss)
        }
      }
    }
  }
}

private struct CameraSnapshotSheet: View {
  @ObservedObject var tile: CameraTile
  let dismiss: () -> Void

  var body: some View {
    NavigationStack {
      VStack(spacing: 0) {
        ZStack {
          Color.black
          if let image = tile.snapshotImage {
            Image(uiImage: image)
              .resizable()
              .scaledToFit()
          }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)

        VStack(alignment: .leading, spacing: 6) {
          Text(tile.snapshotHash)
            .font(.caption2.monospaced())
            .textSelection(.enabled)
          Label(
            tile.snapshotRelayed ? "Fetched through relay" : "Fetched directly",
            systemImage: tile.snapshotRelayed ? "network" : "link"
          )
          .font(.caption)
          .foregroundStyle(.secondary)
        }
        .padding()
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(CameraMeshStyle.surface)
      }
      .background(Color.black)
      .navigationTitle(tile.name)
      .navigationBarTitleDisplayMode(.inline)
      .toolbar {
        ToolbarItem(placement: .confirmationAction) {
          Button("Done", action: dismiss)
        }
      }
    }
  }
}

private struct CameraPublisherView: View {
  @ObservedObject var model: CameraMeshModel

  var body: some View {
    NavigationStack {
      Form {
        Section("Foreground camera publisher") {
          CameraPreview(image: model.latestFrameImage, placeholder: "Waiting for the back camera…")
          HStack {
            Label("\(model.frameCount) captured", systemImage: "photo.stack")
            Spacer()
            Label(
              model.paused ? "Paused" : "Streaming",
              systemImage: model.paused ? "pause.circle" : "dot.radiowaves.left.and.right"
            )
            .foregroundStyle(model.paused ? CameraMeshStyle.warning : CameraMeshStyle.accent)
          }
          .font(.caption.monospacedDigit())
          Text("480×270 JPEG · up to 5 fps · foreground only")
            .font(.caption)
            .foregroundStyle(.secondary)
        }

        Section("Pending viewers") {
          if model.pendingViewerPeerIDs.isEmpty {
            Text("No viewer is waiting for approval.")
              .foregroundStyle(.secondary)
          } else {
            ForEach(model.pendingViewerPeerIDs, id: \.self) { peerID in
              VStack(alignment: .leading, spacing: 8) {
                Text(peerID).font(.caption2.monospaced()).textSelection(.enabled)
                Button("Approve exact Peer ID") {
                  Task { await model.approveViewer(peerID) }
                }
                .buttonStyle(.borderedProminent)
              }
            }
          }
        }

        if !model.approvedViewerPeerIDs.isEmpty {
          Section("Approved viewers") {
            ForEach(model.approvedViewerPeerIDs, id: \.self) { peerID in
              HStack {
                Text(shortCameraPeerID(peerID)).font(.caption.monospaced())
                Spacer()
                Button("Revoke", role: .destructive) {
                  Task { await model.revokeViewer(peerID) }
                }
              }
            }
          }
        }

        Section("This publisher") {
          Text(model.localPeerID)
            .font(.caption2.monospaced())
            .textSelection(.enabled)
          Button("Copy peer card") { UIPasteboard.general.string = model.localCard }
          if !model.lastPublisherEvent.isEmpty {
            LabeledContent("Last event", value: model.lastPublisherEvent)
          }
        }

        Section("Runtime log") {
          Text(model.log.isEmpty ? "Ready" : model.log)
            .font(.caption.monospaced())
            .textSelection(.enabled)
        }

        Section {
          Button("Stop publisher", role: .destructive) {
            Task { await model.stop() }
          }
        }
      }
      .navigationTitle("Camera Publisher")
      .navigationBarTitleDisplayMode(.inline)
    }
  }
}

private struct CameraPreview: View {
  let image: UIImage?
  let placeholder: String

  var body: some View {
    ZStack {
      Color.black
      if let image {
        Image(uiImage: image)
          .resizable()
          .scaledToFit()
      } else {
        VStack(spacing: 8) {
          Image(systemName: "video.slash").font(.title2)
          Text(placeholder).font(.caption)
        }
        .foregroundStyle(.white.opacity(0.65))
      }
    }
    .aspectRatio(16.0 / 9.0, contentMode: .fit)
    .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
  }
}

private struct CameraMeshBrand: View {
  let showName: Bool

  var body: some View {
    HStack(spacing: 10) {
      ZStack {
        RoundedRectangle(cornerRadius: 10, style: .continuous)
          .fill(CameraMeshStyle.surface)
        RoundedRectangle(cornerRadius: 10, style: .continuous)
          .stroke(CameraMeshStyle.accent.opacity(0.25))
        Circle()
          .stroke(CameraMeshStyle.accent, lineWidth: 4)
          .padding(10)
      }
      .frame(width: 44, height: 44)
      .shadow(color: CameraMeshStyle.accent.opacity(0.18), radius: 10)

      if showName {
        (Text("Camera ").foregroundStyle(CameraMeshStyle.text)
          + Text("Mesh").foregroundStyle(CameraMeshStyle.accent))
          .font(.headline.bold())
          .lineLimit(1)
      }
    }
  }
}

private struct CameraMeshPrimaryButtonStyle: ButtonStyle {
  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .font(.headline)
      .padding(.vertical, 14)
      .background(CameraMeshStyle.accent.opacity(configuration.isPressed ? 0.65 : 0.95))
      .foregroundStyle(Color.black.opacity(0.82))
      .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
  }
}

private struct CameraMeshFieldModifier: ViewModifier {
  func body(content: Content) -> some View {
    content
      .padding(.horizontal, 14)
      .frame(minHeight: 50)
      .background(Color.black.opacity(0.24))
      .clipShape(RoundedRectangle(cornerRadius: 11, style: .continuous))
      .overlay {
        RoundedRectangle(cornerRadius: 11, style: .continuous)
          .stroke(CameraMeshStyle.line)
      }
  }
}

extension View {
  fileprivate func cameraMeshField() -> some View {
    modifier(CameraMeshFieldModifier())
  }
}

func effectiveCameraColumnCount(requested: Int, compact: Bool) -> Int {
  let clamped = min(4, max(1, requested))
  return compact && clamped > 1 ? 2 : clamped
}
