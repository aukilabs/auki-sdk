import Foundation

@MainActor
final class CameraStreamerViewModel: ObservableObject {
    @Published var clusterName = "auki-camera"
    @Published var discoveryUrl = "http://127.0.0.1:8091"
    @Published var loggingEnabled = true
    @Published var streamingEnabled = true
    @Published private(set) var peerId = ""
    @Published private(set) var isRunning = false
    @Published private(set) var statusText = "Idle"

    func toggleRunning() async {
        isRunning.toggle()
        if isRunning {
            peerId = "pending"
            statusText = "Running"
        } else {
            peerId = ""
            statusText = "Stopped"
        }
    }
}
