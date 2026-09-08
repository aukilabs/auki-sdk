import SwiftUI
import UIKit

struct ContentView: View {
    @ObservedObject var model: EchoModel

    var body: some View {
        NavigationStack {
            Form {
                Section("Auki User") {
                    TextField("Email", text: $model.email)
                        .textContentType(.username)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    SecureField("Password", text: $model.password)
                        .textContentType(.password)
                    Button("Log in") { Task { await model.login() } }
                        .disabled(!model.canLogin)
                }

                if !model.domains.isEmpty {
                    Section("Domain") {
                        Picker("Domain", selection: $model.selectedDomainID) {
                            ForEach(model.domains, id: \.id) { domain in
                                Text(domain.name.map { "\($0) — \(domain.id)" } ?? domain.id)
                                    .tag(domain.id)
                            }
                        }
                        Picker("Discovery", selection: $model.advertisePeer) {
                            Text("Discover + advertise").tag(true)
                            Text("Discover only (stay hidden)").tag(false)
                        }
                        Button("Start peer") { Task { await model.start() } }
                            .disabled(!model.canStart)
                    }
                }

                if !model.localCard.isEmpty {
                    Section("This peer") {
                        Text(model.localCard)
                            .font(.caption.monospaced())
                            .textSelection(.enabled)
                        Button("Copy peer card") { UIPasteboard.general.string = model.localCard }
                    }

                    Section("Discovered Echo peers") {
                        Text("Candidates are untrusted until the exact dial succeeds.")
                            .font(.caption)
                        Button("Refresh Echo peers") { Task { await model.refreshDiscovery() } }
                            .disabled(!model.canRefreshDiscovery)
                        if !model.discoveredPeers.isEmpty {
                            Picker("Peer", selection: $model.selectedDiscoveredPeerID) {
                                ForEach(model.discoveredPeers, id: \.peerId) { candidate in
                                    Text(candidate.peerId).tag(candidate.peerId)
                                }
                            }
                            TextField("Message", text: $model.message)
                            Button("Send to selected peer") {
                                Task { await model.sendDiscovered() }
                            }
                            .disabled(!model.canSendDiscovered)
                        }
                    }

                    Section("Manual exact-target fallback") {
                        TextEditor(text: $model.remoteCard)
                            .font(.caption.monospaced())
                            .frame(minHeight: 120)
                        TextField("Message", text: $model.message)
                        Button("Send using peer card") { Task { await model.send() } }
                            .disabled(!model.canSend)
                        Button("Stop peer", role: .destructive) { Task { await model.stop() } }
                    }
                }

                Section("Status — \(model.phase.rawValue)") {
                    Text(model.log.isEmpty ? "Ready" : model.log)
                        .font(.caption.monospaced())
                        .textSelection(.enabled)
                }
            }
            .navigationTitle("Auki Portable Echo")
        }
    }
}
