import Combine
import Foundation

@MainActor
final class LoginViewModel: ObservableObject {
    @Published var username: String = ""
    @Published var password: String = ""
    @Published var isLoading: Bool = false
    @Published var loginError: String?
    @Published var isLoggedIn: Bool = false

    func login() async {
        guard !username.isEmpty, !password.isEmpty else {
            loginError = "Please enter both username and password"
            return
        }

        isLoading = true
        defer { isLoading = false }

        let failure = ProcessInfo.processInfo.arguments.contains("--ui_test_login_failure")
        try? await Task.sleep(nanoseconds: 300_000_000)

        if failure {
            loginError = "Invalid credentials"
        } else {
            loginError = nil
            isLoggedIn = true
        }
    }
}
