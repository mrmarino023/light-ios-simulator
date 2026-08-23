import SwiftUI

struct ContentView: View {
    @State private var showSheet = false
    @State private var confirmed = false

    var body: some View {
        NavigationStack {
            VStack(spacing: 24) {
                Text("ModalHome")
                    .font(.largeTitle)
                    .accessibilityIdentifier("ModalHome")
                if confirmed {
                    Text("ModalConfirmed")
                        .font(.title2)
                        .accessibilityIdentifier("ModalConfirmed")
                }
                Button("OpenSheet") {
                    showSheet = true
                }
                .buttonStyle(.borderedProminent)
                .accessibilityIdentifier("OpenSheet")
            }
            .padding()
            .navigationTitle("Modal")
            .sheet(isPresented: $showSheet) {
                NavigationStack {
                    VStack(spacing: 20) {
                        Text("SheetTitle")
                            .font(.title2)
                            .accessibilityIdentifier("SheetTitle")
                        Button("ConfirmAction") {
                            confirmed = true
                            showSheet = false
                        }
                        .buttonStyle(.borderedProminent)
                        .accessibilityIdentifier("ConfirmAction")
                        Button("CancelSheet") {
                            showSheet = false
                        }
                        .accessibilityIdentifier("CancelSheet")
                    }
                    .padding()
                    .navigationTitle("Sheet")
                }
            }
        }
    }
}
