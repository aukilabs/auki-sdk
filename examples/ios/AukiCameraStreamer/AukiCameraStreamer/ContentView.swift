import SwiftUI

struct ContentView: View {
    @StateObject private var viewModel = CameraStreamerViewModel()

    var body: some View {
        NavigationStack {
            Form {
                Section("Cluster") {
                    TextField("Cluster name", text: $viewModel.clusterName)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextField("Discovery URL", text: $viewModel.discoveryUrl)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .keyboardType(.URL)
                }

                Section("Runtime") {
                    Toggle("Logging", isOn: $viewModel.loggingEnabled)
                    Toggle("Streaming", isOn: $viewModel.streamingEnabled)
                }

                Section("Preview") {
                    ZStack {
                        Rectangle()
                            .fill(.black)
                        if let previewImage = viewModel.previewImage {
                            Image(uiImage: previewImage)
                                .resizable()
                                .scaledToFit()
                        } else {
                            Text("No camera frame")
                                .foregroundStyle(.secondary)
                        }
                    }
                    .frame(maxWidth: .infinity)
                    .frame(height: 240)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
                }

                Section("Status") {
                    LabeledContent("Peer", value: viewModel.peerId)
                    LabeledContent("State", value: viewModel.statusText)
                    Button(viewModel.isRunning ? "Stop" : "Start") {
                        Task {
                            await viewModel.toggleRunning()
                        }
                    }
                }
            }
            .navigationTitle("Auki Camera")
        }
    }
}
