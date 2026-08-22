import XCTest

struct HomePage {
    let app: XCUIApplication
    var title: XCUIElement { app.staticTexts["homeTitle"].firstMatch }
    var message: XCUIElement { app.staticTexts["homeMessage"].firstMatch }
}
