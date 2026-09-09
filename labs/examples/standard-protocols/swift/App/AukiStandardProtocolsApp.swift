import SwiftUI

@main
struct AukiStandardProtocolsApp: App {
  @StateObject private var model = StandardProtocolsModel()
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
