import SwiftUI

struct DevicesView: View {
    @ObservedObject var model: SyncModel
    @State private var address = ""
    @State private var code = ""

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
                        Text("\(peer.name)\(peer.isOnline ? " · online" : "")")
                    }
                }
                Section {
                    ForEach(model.nearby, id: \.id) { device in
                        HStack {
                            Text("Quakboard #\(device.id.suffix(4))")
                            Spacer()
                            Button("Pair") { model.pair(withDevice: device.id) }
                        }
                    }
                } header: {
                    Text("Nearby")
                } footer: {
                    if let problem = model.discoveryProblem {
                        Text(problem)
                    }
                }
                if !model.status.isEmpty {
                    Section("Status") {
                        Text(model.status)
                    }
                }
            }
            .scrollContentBackground(.hidden)
            .background(Theme.background)
            .navigationTitle("Devices")
        }
    }
}
