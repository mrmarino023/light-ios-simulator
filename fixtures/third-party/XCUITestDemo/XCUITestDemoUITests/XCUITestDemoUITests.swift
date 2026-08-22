import XCTest

final class XCUITestDemoUITests: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
        UserDefaults.standard.removeObject(forKey: "isLoggedIn")
    }

    func testHappyPathLogin() {
        let page = LoginPage(app: XCUIApplication()).launch()
        page.fill(username: "alice", password: "secret")
        page.submit()

        let home = HomePage(app: page.app)
        XCTAssertTrue(home.title.waitForExistence(timeout: 5.0))
        takeScreenshot(name: "01_Home_Success", app: page.app)
    }

    func testLoginValidationShowsError() {
        let page = LoginPage(app: XCUIApplication()).launch()
        page.submit()

        let error = page.errorLabel
        XCTAssertTrue(error.waitForExistence(timeout: 3.0))
        XCTAssertEqual(error.label, "Please enter both username and password")
        takeScreenshot(name: "02_Validation_Error", app: page.app)
    }

    func testLoginFailurePath() {
        let page = LoginPage(app: XCUIApplication())
            .launch(arguments: ["--ui_test_login_failure"])

        page.fill(username: "bob", password: "wrong")
        page.submit()

        let error = page.errorLabel
        XCTAssertTrue(error.waitForExistence(timeout: 4.0))
        XCTAssertEqual(error.label, "Invalid credentials")
        takeScreenshot(name: "03_Server_Error", app: page.app)
    }

    private func takeScreenshot(name: String, app: XCUIApplication) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
