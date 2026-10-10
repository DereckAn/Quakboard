import SwiftUI

struct ContentView: View {
    @StateObject private var model = SyncModel()
    @State private var address = ""
    @State private var code = ""
    @State private var outgoing = ""

    var body: some View {
        NavigationStack {
            Form {
                Section("This device") {
                    Text(model.identityLine)
                }
                Section("Pair with a desktop") {
                    TextField("Desktop IP, like 192.168.1.20", text: $address)
                        .keyboardType(.numbersAndPunctuation)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Button("Pair") { model.pair(with: address) }
                    if model.isWaitingForCode {
                        TextField("Code", text: $code)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                        Button("Submit code") { model.submit(code: code) }
                    }
                }
                Section("Paired devices") {
                    ForEach(model.peers, id: \.id) { peer in
                        Text(peer.name)
                    }
                }
                Section("Last received text") {
                    Text(model.lastText)
                        .textSelection(.enabled)
                }
                Section("Send text") {
                    TextField("Text", text: $outgoing)
                    Button("Send") { Task { await model.send(outgoing) } }
                }
                if !model.status.isEmpty {
                    Section("Status") {
                        Text(model.status)
                    }
                }
            }
            .navigationTitle("Quakboard")
        }
        .task { await model.start() }
    }
}
