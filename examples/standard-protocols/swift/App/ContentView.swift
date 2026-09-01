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

          Section("Probe another peer") {
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
