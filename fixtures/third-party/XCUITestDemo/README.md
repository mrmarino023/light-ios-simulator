## 1) XCUITestDemo
XCUITestDemo is SwiftUI sample app with XCUITest UI automation using Page Objects, stable accessibility IDs, a test plan, screenshots as attachments

---

## 2) Features

- ✅ SwiftUI state-driven navigation (`@Published isLoggedIn`)
- ✅ Stable accessibility identifiers on leaf views (labels, buttons, fields)
- ✅ Page Object pattern for readable, maintainable tests
- ✅ Launch arguments to simulate server success/failure
- ✅ Screenshots saved as `XCTAttachment` on pass/fail

---

## 3) Project structure

```
XCUITestDemo/
├─ XCUITestDemo/                   # App
│  ├─ XCUITestDemoApp.swift
│  ├─ ContentView.swift            # uses homeTitle/homeMessage identifiers
│  └─ LoginViewModel.swift         # @Published isLoggedIn
└─ XCUITestDemoUITests/            # UI tests
   ├─ XCUITestDemoUITests.swift    # 3 tests + screenshot attachments
   ├─ Pages/
   │  ├─ LoginPage.swift
   │  └─ HomePage.swift
```

---

## 4) Quick start

1. Open `XCUITestDemo.xcodeproj` in Xcode.
2. Pick an iPhone Simulator (e.g., iPhone 15 or 16).
3. Press **⌘U** to run UI tests.
4. Open **Test Navigator** (⌘8) for green/red indicators.

> If using the test plan, select it under **Scheme → Edit Scheme → Test → Test Plans**.

---

## 5) Tests included

- `testHappyPathLogin()` — fills username/password, taps **Login**, waits for `homeTitle` (5s timeout).
- `testLoginValidationShowsError()` — taps **Login** with empty fields, expects validation message.
- `testLoginFailurePath()` — launches app with `--ui_test_login_failure`, expects “Invalid credentials”.

**Selectors used**
```swift
Text("Home").accessibilityIdentifier("homeTitle")
Text("You’re logged in!").accessibilityIdentifier("homeMessage")
```

---
## 6) Reports & attachments

### In Xcode (GUI)
1. After tests finish (⌘U), open **Report Navigator** (⌘9).
2. Select the latest **Test** run.
3. Expand **Tests** → pick a **test method**.
4. In the **Activities** panel, preview **Attachments** (screenshots).
5. Right‑click an attachment → **Save As…** to export PNG/logs.
6. For inline pass/fail in editor: **Editor → Show Test Results**.
