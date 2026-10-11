import SwiftUI

/// Both directions of pairing: typing the other device's code, or showing
/// ours for the other device to type.
struct PairingSheet: View {
    @ObservedObject var model: SyncModel
    let pairing: SyncModel.Pairing

    @State private var code = ""
    @FocusState private var isCodeFocused: Bool

    var body: some View {
        VStack(spacing: 16) {
            switch pairing {
            case .enteringCode:
                heading("Enter the pairing code", systemImage: "lock.shield",
                        detail: "Type the 6-digit code the other device shows.")
                TextField("", text: digits, prompt: Text("000000").foregroundColor(Theme.textMuted))
                    .font(.system(.title, design: .monospaced).weight(.semibold))
                    .foregroundStyle(Theme.text)
                    .multilineTextAlignment(.center)
                    .keyboardType(.numberPad)
                    .textContentType(.oneTimeCode)
                    .focused($isCodeFocused)
                    .padding(.vertical, 10)
                    .background(Theme.inset, in: RoundedRectangle(cornerRadius: 12))
                    .overlay(RoundedRectangle(cornerRadius: 12).stroke(Theme.border))
                    .accessibilityLabel("Pairing code")
                Button("Pair") { model.submit(code: code) }
                    .buttonStyle(ThemeButtonStyle(isPrimary: true))
                    .disabled(code.count != 6)
            case .showingCode(let code):
                heading("Type this code on the other device", systemImage: "number",
                        detail: "Pairing finishes as soon as it's entered there.")
                Text("\(code.prefix(3)) \(code.suffix(3))")
                    .font(.system(size: 40, weight: .bold, design: .monospaced))
                    .tracking(4)
                    .foregroundStyle(Theme.accentText)
                    .textSelection(.enabled)
                    .accessibilityLabel(code.map(String.init).joined(separator: " "))
            }
            Button("Cancel", action: model.cancelPairing)
                .buttonStyle(ThemeButtonStyle())
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Theme.background)
        .presentationDetents([.medium])
        .onAppear { isCodeFocused = true }
    }

    /// Only digits, at most six.
    private var digits: Binding<String> {
        Binding(get: { code }, set: { code = String($0.filter(\.isNumber).prefix(6)) })
    }

    private func heading(_ title: String, systemImage: String, detail: String) -> some View {
        VStack(spacing: 8) {
            Image(systemName: systemImage)
                .font(.system(size: 28))
                .foregroundStyle(Theme.accentText)
                .accessibilityHidden(true)
            Text(title)
                .font(.title3.weight(.semibold))
                .foregroundStyle(Theme.text)
                .multilineTextAlignment(.center)
            Text(detail)
                .font(.subheadline)
                .foregroundStyle(Theme.textSecondary)
                .multilineTextAlignment(.center)
        }
    }
}
