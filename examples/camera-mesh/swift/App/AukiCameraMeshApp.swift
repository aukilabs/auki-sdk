import SwiftUI

@main
struct AukiCameraMeshApp: App {
  @StateObject private var model = CameraMeshModel()
  @Environment(\.scenePhase) private var scenePhase

  var body: some Scene {
    WindowGroup {
      ContentView(model: model)
        .task { model.runAutomationIfConfigured() }
    }
    .onChange(of: scenePhase) { _, phase in
      guard phase == .background else { return }
      Task { await model.stop(reason: "App entered the background") }
    }
  }
}
