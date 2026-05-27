import SwiftUI

@main
struct AukiCameraStreamerApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView(viewModel: CameraStreamerViewModel())
        }
    }
}
