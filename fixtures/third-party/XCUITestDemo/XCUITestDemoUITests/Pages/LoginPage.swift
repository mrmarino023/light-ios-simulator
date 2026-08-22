import XCTest

struct LoginPage {
    let app: XCUIApplication

    var username: XCUIElement { app.textFields["usernameTextField"].firstMatch }
    var password: XCUIElement { app.secureTextFields["passwordSecureField"].firstMatch }
    var loginButton: XCUIElement { app.buttons["loginButton"].firstMatch }
    var errorLabel: XCUIElement { app.staticTexts["loginErrorLabel"].firstMatch }
    var spinner: XCUIElement { app.otherElements["loginProgressView"].firstMatch }

    @discardableResult
    func launch(arguments: [String] = []) -> LoginPage {
        let app = XCUIApplication()
        app.launchArguments = arguments
        app.launch()
        return LoginPage(app: app)
    }

    func fill(username: String, password: String) {
        self.username.tap()
        self.username.typeText(username)
        self.password.tap()
        self.password.typeText(password)
    }

    func submit() { loginButton.tap() }
}
