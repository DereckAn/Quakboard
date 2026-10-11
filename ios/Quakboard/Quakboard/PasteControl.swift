import SwiftUI
import UniformTypeIdentifiers

/// The system Paste button, which doesn't ask "Allow Paste?", in the draft's
/// sand colors. SwiftUI's PasteButton ignores the text color, leaving white
/// on sand. Its label is always the system's "Paste".
struct PasteControl: UIViewRepresentable {
    let onPaste: ([NSItemProvider]) -> Void

    func makeUIView(context: Context) -> UIPasteControl {
        let configuration = UIPasteControl.Configuration()
        configuration.displayMode = .iconAndLabel
        configuration.cornerStyle = .capsule
        configuration.baseBackgroundColor = UIColor(Theme.accent)
        configuration.baseForegroundColor = UIColor(Theme.onAccent)
        let control = UIPasteControl(configuration: configuration)
        control.target = context.coordinator
        return control
    }

    func updateUIView(_ control: UIPasteControl, context: Context) {
        context.coordinator.onPaste = onPaste
    }

    func makeCoordinator() -> Target {
        Target(onPaste: onPaste)
    }

    final class Target: UIResponder {
        var onPaste: ([NSItemProvider]) -> Void

        init(onPaste: @escaping ([NSItemProvider]) -> Void) {
            self.onPaste = onPaste
            super.init()
            pasteConfiguration = UIPasteConfiguration(
                acceptableTypeIdentifiers: [UTType.image.identifier, UTType.text.identifier]
            )
        }

        override func paste(itemProviders: [NSItemProvider]) {
            onPaste(itemProviders)
        }
    }
}
