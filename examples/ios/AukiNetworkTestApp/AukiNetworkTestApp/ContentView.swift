import SwiftUI

struct ContentView: View {
    @StateObject private var model = MessageNodeViewModel()

    var body: some View {
        NavigationStack {
            Form {
                Section("Local") {
                    TextField("Wallet seed hex", text: $model.walletSeedHex)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Button("Start node") { model.start() }
                    Button("Refresh listen addresses") { model.refreshListenAddrs() }
                    Button("Stop node") { model.stop() }
                    Text(model.peerId).font(.footnote).textSelection(.enabled)
                    Text(model.listenAddrs).font(.footnote).textSelection(.enabled)
                }

                Section("Browser peer") {
                    TextField("Browser peer id", text: $model.browserPeerId)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    TextEditor(text: $model.browserAddrs)
                        .frame(minHeight: 80)
                    Button("Dial browser") { model.dialBrowser() }
                    Button("Send ping") { model.sendPing() }
                    Button("Poll event") { model.pollEvent() }
                }

                Section("Log") {
                    Text(model.eventLog)
                        .font(.system(.footnote, design: .monospaced))
                        .textSelection(.enabled)
                }
            }
            .navigationTitle("Auki Network")
        }
    }
}
