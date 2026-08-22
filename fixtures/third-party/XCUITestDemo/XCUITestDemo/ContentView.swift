//
//  ContentView.swift
//  XCUITestDemo
//
//  Created by Himali Marasinghe on 2025-08-31.
//

import SwiftUI

struct ContentView: View {
    @StateObject var viewModel: LoginViewModel

    init(viewModel: LoginViewModel) {
        _viewModel = StateObject(wrappedValue: viewModel)
    }

    var body: some View {
        Group {
            if viewModel.isLoggedIn {
                HomeView()
            } else {
                LoginView()
                    .environmentObject(viewModel)
            }
        }
        .animation(.default, value: viewModel.isLoggedIn)
    }
}

private struct LoginView: View {
    @EnvironmentObject var vm: LoginViewModel

    var body: some View {
        VStack(spacing: 16) {
            Text("Welcome").font(.largeTitle)

            TextField("Username", text: $vm.username)
                .textFieldStyle(.roundedBorder)
                .accessibilityIdentifier("usernameTextField")

            SecureField("Password", text: $vm.password)
                .textFieldStyle(.roundedBorder)
                .accessibilityIdentifier("passwordSecureField")

            if let error = vm.loginError {
                Text(error)
                    .foregroundStyle(.red)
                    .accessibilityIdentifier("loginErrorLabel")
            }

            Button {
                Task { await vm.login() }
            } label: {
                if vm.isLoading {
                    ProgressView().accessibilityIdentifier("loginProgressView")
                } else {
                    Text("Login")
                }
            }
            .buttonStyle(.borderedProminent)
            .accessibilityIdentifier("loginButton")
        }
        .padding()
    }
}

private struct HomeView: View {
    var body: some View {
        VStack(spacing: 12) {
            Text("Home").font(.largeTitle)
                .accessibilityIdentifier("homeTitle")
            Text("You’re logged in!")
                .accessibilityIdentifier("homeMessage")
        }
    }
}
