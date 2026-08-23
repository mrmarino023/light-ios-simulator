import SwiftUI

struct ContentView: View {
    @State private var step = 0
    @State private var name = ""

    var body: some View {
        NavigationStack {
            Group {
                switch step {
                case 0:
                    VStack(spacing: 20) {
                        Text("Welcome")
                            .font(.largeTitle)
                            .accessibilityIdentifier("OnboardWelcome")
                        Text("Get started with the app")
                        Button("Continue") { step = 1 }
                            .buttonStyle(.borderedProminent)
                            .accessibilityIdentifier("OnboardContinue")
                        Button("Skip") { step = 3 }
                            .accessibilityIdentifier("OnboardSkip")
                    }
                case 1:
                    VStack(spacing: 20) {
                        Text("Your name")
                            .font(.title2)
                            .accessibilityIdentifier("OnboardNameTitle")
                        TextField("Name", text: $name)
                            .textFieldStyle(.roundedBorder)
                            .padding(.horizontal)
                            .accessibilityIdentifier("OnboardNameField")
                        Button("Next") { step = 2 }
                            .buttonStyle(.borderedProminent)
                            .accessibilityIdentifier("OnboardNext")
                    }
                case 2:
                    VStack(spacing: 20) {
                        Text("Almost done")
                            .accessibilityIdentifier("OnboardAlmostDone")
                        Button("Finish") { step = 3 }
                            .buttonStyle(.borderedProminent)
                            .accessibilityIdentifier("OnboardFinish")
                    }
                default:
                    VStack(spacing: 16) {
                        Text("HomeReady")
                            .font(.largeTitle)
                            .accessibilityIdentifier("HomeReady")
                        if !name.isEmpty {
                            Text(name).accessibilityIdentifier("HomeUserName")
                        }
                    }
                }
            }
            .padding()
            .navigationTitle("Onboard")
        }
    }
}
