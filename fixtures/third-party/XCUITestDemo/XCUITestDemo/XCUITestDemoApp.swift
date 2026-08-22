//
//  XCUITestDemoApp.swift
//  XCUITestDemo
//
//  Created by Himali Marasinghe on 2025-08-31.
//

import SwiftUI

@main
struct XCUITestDemoApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView(viewModel: LoginViewModel())
        }
    }
}
