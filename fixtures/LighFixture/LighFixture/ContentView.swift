import SwiftUI

struct ContentView: View {
    @State private var name = ""
    @State private var showDone = false
    @FocusState private var nameFocused: Bool

    var body: some View {
        NavigationStack {
            if showDone {
                VStack(spacing: 16) {
                    Text("LighDone")
                        .font(.largeTitle)
                        .accessibilityIdentifier("LighDone")
                        .accessibilityLabel("LighDone")
                    Text(name.isEmpty ? "ok" : name)
                        .accessibilityIdentifier("LighDoneDetail")
                        .accessibilityLabel(name.isEmpty ? "ok" : name)
                    Button("BackHome") {
                        showDone = false
                        name = ""
                    }
                    .accessibilityIdentifier("BackHome")
                    .accessibilityLabel("BackHome")
                }
                .navigationTitle("Done")
            } else {
                VStack(spacing: 20) {
                    Text("LighHome")
                        .font(.largeTitle)
                        .accessibilityIdentifier("LighHome")
                        .accessibilityLabel("LighHome")
                    TextField("Name", text: $name)
                        .textFieldStyle(.roundedBorder)
                        .padding(.horizontal)
                        .focused($nameFocused)
                        .submitLabel(.done)
                        .onSubmit { nameFocused = false }
                        .accessibilityIdentifier("NameField")
                        .accessibilityLabel("NameField")
                    Button("GoNext") {
                        nameFocused = false
                        showDone = true
                    }
                    .buttonStyle(.borderedProminent)
                    .accessibilityIdentifier("GoNext")
                    .accessibilityLabel("GoNext")
                }
                .navigationTitle("Home")
            }
        }
    }
}
