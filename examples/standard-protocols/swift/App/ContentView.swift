import SwiftUI
import UIKit

struct ContentView: View {
  @ObservedObject var model: StandardProtocolsModel

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
            Picker("Discovery", selection: $model.discoveryChoice) {
              ForEach(StandardDiscoveryChoice.allCases) { choice in
                Text(choice.rawValue).tag(choice)
              }
            }
            Button("Start all six protocols") { Task { await model.start() } }
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

          Section("Discover peers") {
            Text("Candidates are untrusted until an exact protocol connection authenticates their Peer ID and Domain.")
              .font(.caption)
            Picker("Protocol", selection: $model.selectedDiscoveryProtocol) {
              Text("All advertised peers").tag("")
              ForEach(model.discoveryProtocols, id: \.self) { protocolID in
                Text(protocolID).tag(protocolID)
              }
            }
            Button("Discover") { Task { await model.discoverPeers() } }
              .disabled(!model.canDiscover)

            if !model.discoveredPeers.isEmpty {
              Picker("Candidate", selection: $model.selectedDiscoveredPeerID) {
                ForEach(model.discoveredPeers) { candidate in
                  Text(
                    candidate.peerID
                      + (model.isProbeable(candidate) ? "" : " — not probeable here")
                  )
                  .tag(candidate.peerID)
                  .disabled(!model.isProbeable(candidate))
                }
              }
              if let candidate = model.discoveredPeers.first(where: {
                $0.peerID == model.selectedDiscoveredPeerID
              }) {
                Text("\(candidate.servedProtocols.count) protocols · expires \(candidate.expiresAt)")
                  .font(.caption.monospaced())
                  .textSelection(.enabled)
              }
              Button("Probe selected peer") { Task { await model.probeDiscovered() } }
                .disabled(!model.canProbeDiscovered)
            }
          }

          Section("Manual peer-card fallback") {
            Text("Paste a card when discovery is unavailable or when testing a private peer.")
              .font(.caption)
            TextEditor(text: $model.remoteCard)
              .font(.caption.monospaced())
              .frame(minHeight: 140)
            Button("Probe all six") { Task { await model.probeAll() } }
              .disabled(!model.canProbe)
            Button("Stop peer", role: .destructive) { Task { await model.stop() } }
          }
        }

        Section("Status — \(model.phase.rawValue)") {
          Text(model.log.isEmpty ? "Ready" : model.log)
            .font(.caption.monospaced())
            .textSelection(.enabled)
        }
      }
      .navigationTitle("Auki Protocols")
    }
  }
}
